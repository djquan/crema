use crema_core::{AssetCandidate, AssetId, ScanEvent, scan_folder};
use crema_image::{
    CandidateFormat, DecodeLimits, DecodeOutcome, Decoder, PreviewSize, classify_candidate,
};
use eframe::egui;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Purpose {
    Thumbnail,
    Viewer,
}
impl Purpose {
    fn size(self) -> PreviewSize {
        PreviewSize::new(match self {
            Self::Thumbnail => 320,
            Self::Viewer => 4096,
        })
        .expect("fixed preview size")
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct JobKey {
    pub generation: u64,
    pub asset: AssetId,
    pub purpose: Purpose,
}

pub(crate) struct Job {
    pub key: JobKey,
    pub path: PathBuf,
    pub format: CandidateFormat,
}

pub(crate) enum Event {
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
    },
}

#[derive(Default)]
struct Demand {
    queue: VecDeque<Job>,
    active: Option<JobKey>,
}

pub(crate) struct Jobs {
    pub receiver: Receiver<Event>,
    sender: SyncSender<Event>,
    demand: Arc<(Mutex<Demand>, Condvar)>,
    stopped: Arc<AtomicBool>,
    decoder: Arc<Decoder>,
    worker: Option<JoinHandle<()>>,
    context: egui::Context,
}

fn publish(
    sender: &SyncSender<Event>,
    mut event: Event,
    context: &egui::Context,
    stopped: &AtomicBool,
) -> bool {
    loop {
        if stopped.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(event) {
            Ok(()) => {
                context.request_repaint();
                return true;
            }
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(value)) => {
                event = value;
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

impl Jobs {
    pub fn new(executable: PathBuf, context: egui::Context) -> Self {
        let (sender, receiver) = mpsc::sync_channel(2);
        let demand = Arc::new((Mutex::new(Demand::default()), Condvar::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let decoder = Arc::new(Decoder::new(executable, DecodeLimits::default()));
        let worker = {
            let (demand, stopped, decoder, sender, context) = (
                demand.clone(),
                stopped.clone(),
                decoder.clone(),
                sender.clone(),
                context.clone(),
            );
            thread::spawn(move || {
                loop {
                    let job = {
                        let (mutex, changed) = &*demand;
                        let mut state = mutex.lock().expect("demand lock");
                        while state.queue.is_empty() && !stopped.load(Ordering::Acquire) {
                            state = changed.wait(state).expect("demand wait");
                        }
                        if stopped.load(Ordering::Acquire) {
                            break;
                        }
                        let job = state.queue.pop_front().expect("nonempty demand");
                        state.active = Some(job.key);
                        job
                    };
                    let outcome = decoder.decode(&job.path, job.format, job.key.purpose.size());
                    if !publish(
                        &sender,
                        Event::Decoded {
                            key: job.key,
                            outcome,
                        },
                        &context,
                        &stopped,
                    ) {
                        break;
                    }
                    let mut state = demand.0.lock().expect("demand lock");
                    state.queue.retain(|queued| queued.key != job.key);
                    state.active = None;
                }
            })
        };
        Self {
            receiver,
            sender,
            demand,
            stopped,
            decoder,
            worker: Some(worker),
            context,
        }
    }

    pub fn scan(&self, root: PathBuf, generation: u64) {
        let (sender, context, stopped) = (
            self.sender.clone(),
            self.context.clone(),
            self.stopped.clone(),
        );
        thread::spawn(move || {
            let mut failures = Vec::new();
            match scan_folder(root, classify_candidate) {
                Err(error) => failures.push(error.to_string()),
                Ok(scan) => {
                    for event in scan {
                        match event {
                            ScanEvent::Candidate(candidate) => {
                                if !publish(
                                    &sender,
                                    Event::Candidate {
                                        generation,
                                        candidate,
                                    },
                                    &context,
                                    &stopped,
                                ) {
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
            publish(
                &sender,
                Event::ScanFinished {
                    generation,
                    failures,
                },
                &context,
                &stopped,
            );
        });
    }

    pub fn replace(&self, jobs: Vec<Job>) {
        let mut state = self.demand.0.lock().expect("demand lock");
        state.queue = jobs
            .into_iter()
            .filter(|job| Some(job.key) != state.active)
            .collect();
        self.demand.1.notify_one();
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.decoder.cancel();
        self.demand.1.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
