use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

use serde_json::{json, Value};
use unzip_core::types::{LogLevel, RunCallback};

use crate::{redact, scrub};

static SEQUENCE: AtomicU64 = AtomicU64::new(1);
const HISTORY_LIMIT: usize = 32;

#[derive(Default)]
struct State {
    status: &'static str,
    done: usize,
    total: usize,
    current: String,
    result: Option<Value>,
    secrets: Vec<String>,
}

pub(crate) struct Job {
    id: String,
    kind: &'static str,
    cancel: AtomicBool,
    state: Mutex<State>,
}

impl RunCallback for Job {
    fn on_log(&self, _msg: &str, _level: LogLevel) {
        // Core pack messages can contain plaintext passwords. Never collect raw logs.
    }
    fn on_progress(&self, done: usize, total: usize, name: &str) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.done = done;
        state.total = total;
        state.current = name.chars().take(512).collect();
    }
    fn should_cancel(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Default)]
pub(crate) struct Jobs {
    entries: Arc<Mutex<VecDeque<Arc<Job>>>>,
}

impl Jobs {
    pub fn start<F>(
        &self,
        kind: &'static str,
        secrets: Vec<String>,
        work: F,
    ) -> Result<Value, String>
    where
        F: FnOnce(&Job) -> Value + Send + 'static,
    {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if entries
            .iter()
            .any(|j| j.state.lock().unwrap_or_else(|p| p.into_inner()).status == "running")
        {
            return Err(
                "Another archive job is running. Query or cancel it before starting another."
                    .into(),
            );
        }
        while entries.len() >= HISTORY_LIMIT {
            entries.pop_front();
        }
        let id = format!(
            "job-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let job = Arc::new(Job {
            id: id.clone(),
            kind,
            cancel: AtomicBool::new(false),
            state: Mutex::new(State {
                status: "running",
                secrets,
                ..State::default()
            }),
        });
        entries.push_back(job.clone());
        tokio::task::spawn_blocking(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&job)));
            let mut state = job.state.lock().unwrap_or_else(|p| p.into_inner());
            match outcome {
                Ok(value) => {
                    state.status = if job.should_cancel() {
                        "cancelled"
                    } else if value
                        .get("failed")
                        .and_then(Value::as_array)
                        .is_some_and(|a| !a.is_empty())
                    {
                        "failed"
                    } else {
                        "completed"
                    };
                    // Include generated passwords in redaction of warning/error strings too.
                    collect_passwords(&value, &mut state.secrets);
                    state.result = Some(value);
                }
                Err(_) => {
                    state.status = "failed";
                    state.result = Some(
                        json!({"error": "Archive worker panicked; see stderr for diagnostics."}),
                    );
                }
            }
        });
        Ok(json!({"job_id": id, "status": "running", "kind": kind}))
    }

    pub fn status(&self, id: &str, include_passwords: bool) -> Result<Value, String> {
        let job = self.find(id)?;
        let state = job.state.lock().unwrap_or_else(|p| p.into_inner());
        let mut value = json!({"job_id": job.id, "kind": job.kind, "status": state.status,
            "cancel_requested": job.should_cancel(),
            "progress": {"done": state.done,"total": state.total,"current": state.current},
            "result": state.result});
        if !include_passwords {
            redact(&mut value);
            // Preserve job IDs and terminal status even for one-character passwords.
            scrub(&mut value["result"], &state.secrets);
            scrub(&mut value["progress"]["current"], &state.secrets);
        }
        Ok(value)
    }

    fn find(&self, id: &str) -> Result<Arc<Job>, String> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|job| job.id == id)
            .cloned()
            .ok_or_else(|| "Unknown or expired job_id".into())
    }

    pub fn cancel(&self, id: &str) -> Result<Value, String> {
        let job = self.find(id)?;
        let state = job.state.lock().unwrap_or_else(|p| p.into_inner());
        let status = state.status;
        if status == "running" {
            job.cancel.store(true, Ordering::Relaxed);
        }
        Ok(
            json!({"job_id": id,"status": status,"cancel_requested": job.should_cancel(),
            "note": "Cancellation is cooperative and waits for the current archive tool invocation to return."}),
        )
    }

    pub async fn shutdown(&self) {
        loop {
            let running = {
                let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
                entries
                    .iter()
                    .filter(|j| {
                        j.state.lock().unwrap_or_else(|p| p.into_inner()).status == "running"
                    })
                    .inspect(|j| j.cancel.store(true, Ordering::Relaxed))
                    .count()
            };
            if running == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

fn collect_passwords(value: &Value, secrets: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if key.eq_ignore_ascii_case("password") {
                    if let Some(s) = value.as_str().filter(|s| !s.is_empty()) {
                        secrets.push(s.to_owned());
                    }
                } else {
                    collect_passwords(value, secrets);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_passwords(item, secrets);
            }
        }
        _ => (),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn finish(jobs: &Jobs, id: &str) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let state = jobs.status(id, false).unwrap();
                if state["status"] != "running" {
                    return state;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("worker should finish")
    }
    #[tokio::test]
    async fn cancellation_keeps_busy_until_worker_returns() {
        let jobs = Jobs::default();
        let (sender, receiver) = std::sync::mpsc::channel();
        let started = jobs
            .start("test", vec![], move |callback| {
                receiver.recv().unwrap();
                assert!(callback.should_cancel());
                json!({"ok":[],"failed":[]})
            })
            .unwrap();
        let id = started["job_id"].as_str().unwrap();
        assert!(jobs.start("test", vec![], |_| json!({})).is_err());
        assert_eq!(jobs.cancel(id).unwrap()["status"], "running");
        assert_eq!(jobs.status(id, false).unwrap()["status"], "running");
        sender.send(()).unwrap();
        assert_eq!(finish(&jobs, id).await["status"], "cancelled");
        assert!(jobs.start("test", vec![], |_| json!({})).is_ok());
        jobs.shutdown().await;
    }
    #[tokio::test]
    async fn panicking_worker_releases_slot() {
        let jobs = Jobs::default();
        let first = jobs
            .start("test", vec![], |_| panic!("synthetic worker failure"))
            .unwrap();
        assert_eq!(
            finish(&jobs, first["job_id"].as_str().unwrap()).await["status"],
            "failed"
        );
        assert!(jobs.start("test", vec![], |_| json!({})).is_ok());
        jobs.shutdown().await;
    }
    #[tokio::test]
    async fn generated_password_hidden_unless_requested() {
        let jobs = Jobs::default();
        let first=jobs.start("pack",vec![],|_|json!({"ok":[{"password":"generated-secret"}],"warnings":["generated-secret retained"],"failed":[]})).unwrap();
        let id = first["job_id"].as_str().unwrap();
        assert!(!finish(&jobs, id)
            .await
            .to_string()
            .contains("generated-secret"));
        assert!(jobs
            .status(id, true)
            .unwrap()
            .to_string()
            .contains("generated-secret"));
        assert!(jobs.cancel("missing").is_err());
    }

    #[tokio::test]
    async fn short_password_does_not_corrupt_protocol_state() {
        let jobs = Jobs::default();
        let first = jobs
            .start("pack", vec!["a".into()], |_| json!({"ok":[],"failed":[]}))
            .unwrap();
        let id = first["job_id"].as_str().unwrap();
        let status = finish(&jobs, id).await;
        assert_eq!(status["status"], "completed");
        assert_eq!(status["job_id"], id);
        assert_eq!(status["kind"], "pack");
    }
}
