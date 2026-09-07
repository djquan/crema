use crate::{
    SourceStamp,
    metrics::{Metrics, Span},
    preview::PreviewEngine,
    thumbnail_cache::CacheConfig,
};
use crema_core::{AssetCandidate, AssetId, ScanEvent, scan_folder};
use crema_image::{
    CancelToken, CandidateFormat, DecodeOutcome, FailureClass, PreviewSize, classify_candidate,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Purpose {
    Thumbnail,
    Viewer,
}
impl Purpose {
    pub fn size(self) -> PreviewSize {
        PreviewSize::new(match self {
            Self::Thumbnail => 320,
            Self::Viewer => 4096,
        })
        .expect("fixed preview size")
    }
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct JobKey {
    pub generation: u64,
    pub asset: AssetId,
    pub purpose: Purpose,
}
#[derive(Clone, Debug)]
pub struct PreviewRequest {
    pub key: JobKey,
    pub path: PathBuf,
    pub format: CandidateFormat,
    pub needed: bool,
}
#[derive(Default)]
pub struct PreviewDemand {
    pub selected: Option<PreviewRequest>,
    pub thumbnails: Vec<PreviewRequest>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterestId(pub u64);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptId(pub u64);
pub enum Event {
    Candidate {
        generation: u64,
        candidate: AssetCandidate<CandidateFormat>,
    },
    ScanFinished {
        generation: u64,
        failures: Vec<String>,
    },
    Decoded {
        key: JobKey,
        outcome: DecodeOutcome,
        source_stamp: Option<SourceStamp>,
    },
}
struct Interest {
    id: InterestId,
    request: PreviewRequest,
    done: bool,
    attempt: Option<AttemptId>,
}
#[derive(Clone)]
struct Attempt {
    key: JobKey,
    interest: InterestId,
    id: AttemptId,
    selected: bool,
    cancel: CancelToken,
}
struct Completion {
    attempt: Attempt,
    event: Event,
}
#[derive(Default)]
struct State {
    stopping: bool,
    next_interest: u64,
    next_attempt: u64,
    interests: HashMap<JobKey, Interest>,
    order: Vec<JobKey>,
    selected: Option<JobKey>,
    active: Option<Attempt>,
    selected_ready: Option<Completion>,
    thumbnail_ready: Option<Completion>,
}
impl State {
    fn current(&self, attempt: &Attempt) -> bool {
        !self.stopping
            && self.interests.get(&attempt.key).is_some_and(|interest| {
                interest.id == attempt.interest && interest.attempt == Some(attempt.id)
            })
    }
    fn pending_selected(&self) -> bool {
        self.selected
            .and_then(|key| self.interests.get(&key))
            .is_some_and(|interest| !interest.done && interest.request.needed)
    }
    fn replace(&mut self, demand: PreviewDemand) {
        self.selected = demand.selected.as_ref().map(|request| request.key);
        self.order.clear();
        for request in demand.selected.into_iter().chain(demand.thumbnails) {
            if self.order.contains(&request.key) {
                continue;
            }
            self.order.push(request.key);
            match self.interests.get_mut(&request.key) {
                Some(interest) => {
                    if !interest.request.needed && request.needed {
                        interest.done = false;
                    }
                    interest.request = request;
                }
                None => {
                    self.next_interest += 1;
                    self.interests.insert(
                        request.key,
                        Interest {
                            id: InterestId(self.next_interest),
                            done: !request.needed,
                            request,
                            attempt: None,
                        },
                    );
                }
            }
        }
        self.interests.retain(|key, _| self.order.contains(key));
        if self
            .selected_ready
            .as_ref()
            .is_some_and(|ready| Some(ready.attempt.key) != self.selected)
            && let Some(ready) = self.selected_ready.take()
            && let Some(interest) = self.interests.get_mut(&ready.attempt.key)
        {
            interest.done = false;
        }
        if self
            .selected_ready
            .as_ref()
            .is_some_and(|ready| !self.current(&ready.attempt))
        {
            self.selected_ready = None;
        }
        if self
            .thumbnail_ready
            .as_ref()
            .is_some_and(|ready| !self.current(&ready.attempt))
        {
            self.thumbnail_ready = None;
        }
        if let Some(active) = &self.active
            && (!self.current(active)
                || (self.pending_selected() && self.selected != Some(active.key)))
        {
            active.cancel.cancel();
        }
    }
    fn take(&mut self) -> Option<(Attempt, PreviewRequest)> {
        let key = *self.order.iter().find(|key| {
            self.interests
                .get(key)
                .is_some_and(|interest| !interest.done && interest.request.needed)
        })?;
        let interest = self.interests.get_mut(&key).expect("ordered interest");
        self.next_attempt += 1;
        let attempt = Attempt {
            key,
            interest: interest.id,
            id: AttemptId(self.next_attempt),
            selected: self.selected == Some(key),
            cancel: CancelToken::new(),
        };
        interest.attempt = Some(attempt.id);
        self.active = Some(attempt.clone());
        Some((attempt, interest.request.clone()))
    }
}
type Shared = Arc<(Mutex<State>, Condvar)>;
type Wake = Arc<dyn Fn() + Send + Sync>;
pub struct PreviewRuntime {
    scan_receiver: Receiver<Event>,
    scan_sender: SyncSender<Event>,
    state: Shared,
    worker: Option<JoinHandle<()>>,
    wake: Wake,
    metrics: Metrics,
}

struct DecodeDelivery {
    purpose: Purpose,
    outcome: DecodeOutcome,
    source_stamp: Option<SourceStamp>,
    terminal: bool,
}

fn admit(
    shared: &Shared,
    attempt: &Attempt,
    delivery: DecodeDelivery,
    wake: &Wake,
    span: &Span,
) -> bool {
    let DecodeDelivery {
        purpose,
        outcome,
        source_stamp,
        terminal,
    } = delivery;
    let mut state = shared.0.lock().expect("preview state");
    if matches!(&outcome, DecodeOutcome::Failed(error) if error.class == FailureClass::Cancelled) {
        return false;
    }
    loop {
        if !state.current(attempt) || attempt.cancel.is_cancelled() {
            span.record("obsolete_drop", 1);
            return false;
        }
        let occupied = if attempt.selected {
            state.selected_ready.is_some()
        } else {
            state.thumbnail_ready.is_some()
        };
        if !occupied {
            break;
        }
        if !attempt.selected && state.pending_selected() {
            span.record("priority_drop", 1);
            return false;
        }
        span.record("admission_wait", 1);
        state = shared.1.wait(state).expect("preview admission");
    }
    let output_span = Span {
        key: JobKey {
            purpose,
            ..span.key
        },
        ..span.clone()
    };
    output_span.record(
        if matches!(&outcome, DecodeOutcome::Decoded(_)) {
            if purpose == Purpose::Viewer {
                "viewer_ready"
            } else {
                "thumbnail_ready"
            }
        } else {
            "preview_failed"
        },
        1,
    );
    let completion = Completion {
        attempt: attempt.clone(),
        event: Event::Decoded {
            key: JobKey {
                purpose,
                ..attempt.key
            },
            outcome,
            source_stamp,
        },
    };
    if attempt.selected {
        state.selected_ready = Some(completion);
    } else {
        state.thumbnail_ready = Some(completion);
    }
    state
        .interests
        .get_mut(&attempt.key)
        .expect("current interest")
        .done = terminal;
    drop(state);
    wake();
    true
}
impl PreviewRuntime {
    pub fn new(executable: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_options(executable, CacheConfig::default(), Metrics::default(), wake)
    }
    pub fn with_options(
        executable: PathBuf,
        cache: CacheConfig,
        metrics: Metrics,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (scan_sender, scan_receiver) = mpsc::sync_channel(32);
        let state = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let wake: Wake = Arc::new(wake);
        let worker = {
            let (shared, wake, metrics) = (state.clone(), wake.clone(), metrics.clone());
            thread::spawn(move || {
                let engine = PreviewEngine::new(executable, cache);
                loop {
                    let (attempt, request) = {
                        let mut state = shared.0.lock().expect("preview state");
                        loop {
                            if state.stopping {
                                return;
                            }
                            if let Some(work) = state.take() {
                                break work;
                            }
                            state = shared.1.wait(state).expect("preview demand");
                        }
                    };
                    let span = Span {
                        metrics: metrics.clone(),
                        key: attempt.key,
                        interest: attempt.interest.0,
                        attempt: attempt.id.0,
                    };
                    span.record("attempt_started", 1);
                    engine.run(
                        &request,
                        &attempt.cancel,
                        &span,
                        |purpose, outcome, source_stamp, terminal| {
                            admit(
                                &shared,
                                &attempt,
                                DecodeDelivery {
                                    purpose,
                                    outcome,
                                    source_stamp,
                                    terminal,
                                },
                                &wake,
                                &span,
                            )
                        },
                    );
                    span.record(
                        if attempt.cancel.is_cancelled() {
                            "attempt_cancelled"
                        } else {
                            "attempt_finished"
                        },
                        1,
                    );
                    let mut state = shared.0.lock().expect("preview state");
                    if state
                        .active
                        .as_ref()
                        .is_some_and(|active| active.id == attempt.id)
                    {
                        state.active = None;
                    }
                }
            })
        };
        Self {
            scan_receiver,
            scan_sender,
            state,
            worker: Some(worker),
            wake,
            metrics,
        }
    }
    pub fn replace(&self, demand: PreviewDemand) {
        let mut state = self.state.0.lock().expect("preview state");
        let before = state.next_interest;
        let buffered = [
            state.selected_ready.as_ref(),
            state.thumbnail_ready.as_ref(),
        ]
        .map(|ready| ready.map(|ready| ready.attempt.clone()));
        let was_cancelled = state
            .active
            .as_ref()
            .is_some_and(|active| active.cancel.is_cancelled());
        state.replace(demand);
        for attempt in buffered.into_iter().flatten() {
            if ![
                state.selected_ready.as_ref(),
                state.thumbnail_ready.as_ref(),
            ]
            .into_iter()
            .flatten()
            .any(|ready| ready.attempt.id == attempt.id)
            {
                self.metrics.record(
                    if state.current(&attempt) {
                        "priority_drop"
                    } else {
                        "obsolete_drop"
                    },
                    Some(attempt.key),
                    attempt.interest.0,
                    attempt.id.0,
                    1,
                );
            }
        }
        for interest in state
            .interests
            .values()
            .filter(|interest| interest.id.0 > before)
        {
            self.metrics.record(
                "interest_requested",
                Some(interest.request.key),
                interest.id.0,
                0,
                1,
            );
        }
        if !was_cancelled
            && let Some(active) = &state.active
            && active.cancel.is_cancelled()
        {
            self.metrics.record(
                "cancel_requested",
                Some(active.key),
                active.interest.0,
                active.id.0,
                1,
            );
        }
        self.state.1.notify_all();
    }
    pub fn forget(&self, keys: &[JobKey]) {
        let mut state = self.state.0.lock().expect("preview state");
        for key in keys {
            state.interests.remove(key);
        }
        if let Some(active) = &state.active
            && !state.current(active)
        {
            active.cancel.cancel();
        }
        self.state.1.notify_all();
    }
    pub fn try_recv(&self) -> Option<Event> {
        let mut state = self.state.0.lock().expect("preview state");
        for selected in [true, false] {
            let ready = if selected {
                state.selected_ready.take()
            } else {
                state.thumbnail_ready.take()
            };
            if let Some(ready) = ready {
                self.state.1.notify_all();
                if state.current(&ready.attempt) {
                    return Some(ready.event);
                }
            }
        }
        drop(state);
        self.scan_receiver.try_recv().ok()
    }
    pub fn scan(&self, root: PathBuf, generation: u64) {
        let (sender, shared, wake) = (
            self.scan_sender.clone(),
            self.state.clone(),
            self.wake.clone(),
        );
        thread::spawn(move || {
            let publish = |mut event| loop {
                if shared.0.lock().expect("preview state").stopping {
                    return false;
                }
                match sender.try_send(event) {
                    Ok(()) => {
                        wake();
                        return true;
                    }
                    Err(TrySendError::Disconnected(_)) => return false,
                    Err(TrySendError::Full(returned)) => {
                        event = returned;
                        thread::sleep(Duration::from_millis(2));
                    }
                }
            };
            let mut failures = Vec::new();
            match scan_folder(root, classify_candidate) {
                Err(error) => failures.push(error.to_string()),
                Ok(scan) => {
                    for event in scan {
                        match event {
                            ScanEvent::Candidate(candidate) => {
                                if !publish(Event::Candidate {
                                    generation,
                                    candidate,
                                }) {
                                    return;
                                }
                            }
                            ScanEvent::Failure(error) => {
                                if failures.len() < 32 {
                                    failures.push(error.to_string());
                                }
                            }
                        }
                    }
                }
            }
            publish(Event::ScanFinished {
                generation,
                failures,
            });
        });
    }
}
impl PreviewRuntime {
    pub fn shutdown(&mut self) {
        {
            let mut state = self.state.0.lock().expect("preview state");
            state.stopping = true;
            state.interests.clear();
            state.selected_ready = None;
            state.thumbnail_ready = None;
            if let Some(active) = &state.active {
                if !active.cancel.is_cancelled() {
                    self.metrics.record(
                        "cancel_requested",
                        Some(active.key),
                        active.interest.0,
                        active.id.0,
                        1,
                    );
                }
                active.cancel.cancel();
            }
            self.state.1.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for PreviewRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(generation: u64, purpose: Purpose) -> PreviewRequest {
        let root = std::env::temp_dir().join(format!(
            "crema-scheduler-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("photo.jpg");
        if !path.exists() {
            std::fs::write(&path, []).unwrap();
        }
        let ScanEvent::Candidate(candidate) = scan_folder(&root, classify_candidate)
            .unwrap()
            .next()
            .unwrap()
        else {
            panic!("candidate");
        };
        PreviewRequest {
            key: JobKey {
                generation,
                asset: candidate.id(),
                purpose,
            },
            path,
            format: *candidate.kind(),
            needed: true,
        }
    }
    #[test]
    fn aba_preserves_new_interest_and_preempted_thumbnail_resumes() {
        let a = request(1, Purpose::Thumbnail);
        let b = request(2, Purpose::Viewer);
        let mut state = State::default();
        state.replace(PreviewDemand {
            thumbnails: vec![a.clone()],
            ..Default::default()
        });
        let (a1, _) = state.take().unwrap();
        state.replace(PreviewDemand {
            selected: Some(b.clone()),
            thumbnails: vec![a.clone()],
        });
        assert!(a1.cancel.is_cancelled());
        assert_eq!(state.interests[&a.key].id, a1.interest);
        let (b1, _) = state.take().unwrap();
        state.interests.get_mut(&b.key).unwrap().done = true;
        let (a2, _) = state.take().unwrap();
        assert_eq!(a1.interest, a2.interest);
        assert_ne!(a1.id, a2.id);
        state.replace(PreviewDemand {
            selected: Some(b),
            ..Default::default()
        });
        state.replace(PreviewDemand {
            selected: Some(a.clone()),
            ..Default::default()
        });
        let (a3, _) = state.take().unwrap();
        assert_ne!(a1.interest, a3.interest);
        assert!(!state.current(&a1));
        assert!(!state.current(&a2));
        assert!(!state.current(&b1));
        assert!(state.current(&a3));
        std::fs::remove_dir_all(a.path.parent().unwrap()).unwrap();
    }
    #[test]
    fn removed_interest_never_publishes_its_old_completion() {
        let request = request(1, Purpose::Thumbnail);
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode(
                &vec![90; 3000 * 3000 * 3],
                3000,
                3000,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        std::fs::write(&request.path, bytes).unwrap();
        let jobs = PreviewRuntime::new(std::env::current_exe().unwrap(), || {});
        jobs.replace(PreviewDemand {
            thumbnails: vec![request.clone()],
            ..Default::default()
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while jobs.state.0.lock().unwrap().active.is_none() {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        jobs.replace(PreviewDemand::default());
        while jobs.state.0.lock().unwrap().active.is_some() {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(
            jobs.try_recv().is_none(),
            "removed interest delivered stale completion"
        );
        drop(jobs);
        std::fs::remove_dir_all(request.path.parent().unwrap()).unwrap();
    }
    #[test]
    fn selected_slot_drains_first_and_idle_shutdown_joins() {
        let a = request(1, Purpose::Thumbnail);
        let b = request(2, Purpose::Viewer);
        let jobs = PreviewRuntime::new(std::env::current_exe().unwrap(), || {});
        let mut state = jobs.state.0.lock().unwrap();
        state.replace(PreviewDemand {
            selected: Some(b.clone()),
            thumbnails: vec![a.clone()],
        });
        for request in [&b, &a] {
            let (attempt, _) = state.take().unwrap();
            state.interests.get_mut(&request.key).unwrap().done = true;
            let completion = Completion {
                attempt,
                event: Event::Decoded {
                    key: request.key,
                    outcome: DecodeOutcome::Unsupported("test state".into()),
                    source_stamp: None,
                },
            };
            if request.key == b.key {
                state.selected_ready = Some(completion);
            } else {
                state.thumbnail_ready = Some(completion);
            }
        }
        drop(state);
        assert!(matches!(jobs.try_recv(), Some(Event::Decoded { key, .. }) if key == b.key));
        assert!(matches!(jobs.try_recv(), Some(Event::Decoded { key, .. }) if key == a.key));
        drop(jobs);
        std::fs::remove_dir_all(a.path.parent().unwrap()).unwrap();
    }
    #[test]
    fn previous_selected_completion_cannot_occupy_new_selected_slot() {
        let a = request(1, Purpose::Thumbnail);
        let b = request(2, Purpose::Viewer);
        let mut state = State::default();
        state.replace(PreviewDemand {
            selected: Some(a.clone()),
            ..Default::default()
        });
        let (attempt, _) = state.take().unwrap();
        state.interests.get_mut(&a.key).unwrap().done = true;
        state.selected_ready = Some(Completion {
            attempt,
            event: Event::Decoded {
                key: a.key,
                outcome: DecodeOutcome::Unsupported("pure completion state".into()),
                source_stamp: None,
            },
        });
        state.replace(PreviewDemand {
            selected: Some(b),
            thumbnails: vec![a.clone()],
        });
        assert!(
            state.selected_ready.is_none(),
            "old selected thumbnail cannot block the new selection"
        );
        assert!(!state.interests[&a.key].done);
        std::fs::remove_dir_all(a.path.parent().unwrap()).unwrap();
    }
    #[test]
    fn cached_selection_preserves_visible_work_but_removed_work_still_cancels() {
        let a = request(1, Purpose::Thumbnail);
        let mut b = request(2, Purpose::Thumbnail);
        b.needed = false;
        let mut state = State::default();
        state.replace(PreviewDemand {
            selected: Some(a.clone()),
            ..Default::default()
        });
        let (active, _) = state.take().unwrap();
        state.replace(PreviewDemand {
            selected: Some(b.clone()),
            thumbnails: vec![a.clone()],
        });
        assert!(
            !active.cancel.is_cancelled(),
            "cached selection must preserve still-desired visible work"
        );
        assert_eq!(state.interests[&a.key].id, active.interest);
        assert!(state.interests[&b.key].done);
        state.replace(PreviewDemand {
            selected: Some(b.clone()),
            ..Default::default()
        });
        assert!(
            active.cancel.is_cancelled(),
            "removed thumbnail must cancel"
        );
        let mut viewer = a.clone();
        viewer.key.purpose = Purpose::Viewer;
        state.replace(PreviewDemand {
            selected: Some(viewer),
            ..Default::default()
        });
        let (active_viewer, _) = state.take().unwrap();
        state.replace(PreviewDemand {
            selected: Some(b),
            thumbnails: vec![a.clone()],
        });
        assert!(
            active_viewer.cancel.is_cancelled(),
            "obsolete viewer must cancel even when its successor is cached"
        );
        std::fs::remove_dir_all(a.path.parent().unwrap()).unwrap();
    }
}
