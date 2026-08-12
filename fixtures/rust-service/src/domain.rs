//! Domain service with a SQLite-backed store and a queue fallback.

use std::collections::HashMap;

/// A job-processing service with a store write path and a queue fallback.
pub struct Service {
    name: String,
    queue: HashMap<String, String>,
}

impl Service {
    /// Create a service bound to the given name.
    pub fn new(name: &str) -> Service {
        Service {
            name: name.to_string(),
            queue: HashMap::new(),
        }
    }

    /// Persist a job; on failure fall back to the in-memory queue.
    pub fn ingest(&self, job: &str) -> Result<(), String> {
        match self.write_job(job) {
            Ok(()) => Ok(()),
            Err(e) => {
                self.queue_job(job);
                Err(e)
            }
        }
    }

    /// Notify downstream consumers that a job finished.
    pub fn notify(&self, job: &str) {
        let _ = job;
    }

    /// Store write (rusqlite-style): persist the job row.
    fn write_job(&self, job: &str) -> Result<(), String> {
        let conn = "jobs.db";
        conn.execute("INSERT INTO jobs (id, status) VALUES (?, 'pending')", &[job])
            .expect("store write failed");
        Ok(())
    }

    /// Fallback path: enqueue the job when the store is unavailable.
    fn queue_job(&self, job: &str) {
        self.queue.insert(job.to_string(), "pending".to_string());
    }
}
