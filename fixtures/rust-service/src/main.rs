//! Rust service demo: job ingestion with retry and fanout.

mod domain;

use domain::Service;

/// Entrypoint: process one job through the retryable fanout.
fn main() {
    let svc = Service::new("rust-service");
    publish_with_retry(&svc);
}

/// Retryable fanout: publish a job to both the store and the notify sink,
/// retrying transient failures.
#[retry(attempts = 3, backoff = "exponential")]
fn publish_with_retry(svc: &Service) {
    svc.ingest("job-42");
    svc.notify("job-42");
}
