use crate::jobs::JobKey;
use std::{
    io::{self, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

#[derive(Clone)]
pub struct Metrics(Option<Arc<Log>>);
struct Log {
    session_id: String,
    start: Instant,
    records: Mutex<Vec<Record>>,
    dropped: AtomicU64,
}
#[derive(Clone, Debug)]
pub struct Record {
    pub micros: u64,
    pub event: &'static str,
    pub key: Option<JobKey>,
    pub interest: u64,
    pub attempt: u64,
    pub value: u64,
}
impl Metrics {
    pub fn new(enabled: bool) -> Self {
        Self::with_session(enabled, format!("local-{}", std::process::id()))
    }

    pub fn with_session(enabled: bool, session_id: String) -> Self {
        Self(enabled.then(|| {
            Arc::new(Log {
                session_id,
                start: Instant::now(),
                records: Mutex::new(Vec::new()),
                dropped: AtomicU64::new(0),
            })
        }))
    }
    pub fn record(
        &self,
        event: &'static str,
        key: Option<JobKey>,
        interest: u64,
        attempt: u64,
        value: u64,
    ) {
        if let Some(log) = &self.0 {
            let mut records = log.records.lock().expect("metrics log");
            if records.len() < 100_000 {
                records.push(Record {
                    micros: log.start.elapsed().as_micros() as u64,
                    event,
                    key,
                    interest,
                    attempt,
                    value,
                });
            } else {
                log.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    pub fn records(&self) -> Vec<Record> {
        self.0
            .as_ref()
            .map(|log| log.records.lock().expect("metrics log").clone())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut output = std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)?,
        );
        writeln!(
            output,
            "session_id\tmicros\tevent\tasset\tpurpose\tgeneration\tinterest\tattempt\tvalue"
        )?;
        for record in self.records() {
            let (asset, purpose, generation) = record
                .key
                .map(|key| {
                    (
                        key.asset.to_string(),
                        format!("{:?}", key.purpose),
                        key.generation,
                    )
                })
                .unwrap_or((String::new(), String::new(), 0));
            writeln!(
                output,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                self.0.as_ref().expect("enabled metrics log").session_id,
                record.micros,
                record.event,
                asset,
                purpose,
                generation,
                record.interest,
                record.attempt,
                record.value
            )?;
        }
        if let Some(log) = &self.0 {
            writeln!(
                output,
                "{}\t{}\tmetrics_dropped\t\t\t0\t0\t0\t{}",
                log.session_id,
                log.start.elapsed().as_micros(),
                log.dropped.load(Ordering::Relaxed)
            )?;
        }
        output.flush()
    }
}
impl Default for Metrics {
    fn default() -> Self {
        Self::new(false)
    }
}

#[derive(Clone)]
pub(crate) struct Span {
    pub metrics: Metrics,
    pub key: JobKey,
    pub interest: u64,
    pub attempt: u64,
}
impl Span {
    pub fn record(&self, event: &'static str, value: u64) {
        self.metrics
            .record(event, Some(self.key), self.interest, self.attempt, value);
    }
}
