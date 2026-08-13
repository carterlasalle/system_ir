// tokio-family: impl constructors (`new_multi_thread`) + fluent builder
// chain (`with_worker_threads`) + crate-level `static` (mutable state).

static MAX_BLOCKING: usize = 512;

/// A thread-pool builder, tokio-style.
pub struct Builder {
    threads: usize,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { threads: 1 }
    }

    pub fn new_multi_thread() -> Builder {
        Builder { threads: 4 }
    }

    pub fn with_worker_threads(mut self, n: usize) -> Self {
        self.threads = n;
        self
    }

    pub fn set_thread_name(&mut self, _name: &str) -> &mut Self {
        self
    }

    pub fn build(self) -> Builder {
        self
    }
}
