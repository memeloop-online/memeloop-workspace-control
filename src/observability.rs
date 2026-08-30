use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

mod process;
pub use process::{AllocatorSnapshot, ProcessSnapshot};
use process::{allocator_snapshot, process_snapshot};

pub const HTTP_DURATION_BUCKETS: [f64; 9] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0];

#[derive(Clone)]
pub struct Observability {
    inner: Arc<Inner>,
}

struct Inner {
    started_at: Instant,
    http: Mutex<BTreeMap<HttpKey, HttpSeries>>,
    active_http: AtomicU64,
    request_buffer_bytes: AtomicU64,
    active_sse: AtomicU64,
    sse_buffer_bytes: AtomicU64,
    upstream: [UpstreamCounters; 3],
    cpu_profile_active: AtomicBool,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct HttpKey {
    method: &'static str,
    route: String,
}

#[derive(Clone, Debug, Default)]
struct HttpSeries {
    requests_by_status: BTreeMap<&'static str, u64>,
    duration_count: u64,
    duration_sum: f64,
    duration_buckets: [u64; HTTP_DURATION_BUCKETS.len()],
    request_bytes_total: u64,
    response_bytes_total: u64,
}

struct UpstreamCounters {
    active: AtomicU64,
    success: AtomicU64,
    error: AtomicU64,
}

impl UpstreamCounters {
    fn new() -> Self {
        Self {
            active: AtomicU64::new(0),
            success: AtomicU64::new(0),
            error: AtomicU64::new(0),
        }
    }
}

impl Default for Observability {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                started_at: Instant::now(),
                http: Mutex::new(BTreeMap::new()),
                active_http: AtomicU64::new(0),
                request_buffer_bytes: AtomicU64::new(0),
                active_sse: AtomicU64::new(0),
                sse_buffer_bytes: AtomicU64::new(0),
                upstream: std::array::from_fn(|_| UpstreamCounters::new()),
                cpu_profile_active: AtomicBool::new(false),
            }),
        }
    }
}

impl Observability {
    pub fn begin_http(
        &self,
        method: &str,
        route: impl Into<String>,
        request_bytes: u64,
    ) -> HttpRequestGuard {
        self.inner.active_http.fetch_add(1, Ordering::Relaxed);
        self.inner
            .request_buffer_bytes
            .fetch_add(request_bytes, Ordering::Relaxed);
        HttpRequestGuard {
            observability: self.clone(),
            key: HttpKey {
                method: normalized_method(method),
                route: route.into(),
            },
            started_at: Instant::now(),
            request_bytes,
            finished: false,
        }
    }

    pub fn begin_sse(&self) -> StreamGuard {
        self.inner.active_sse.fetch_add(1, Ordering::Relaxed);
        StreamGuard {
            observability: self.clone(),
            buffer_bytes: 0,
        }
    }

    pub fn begin_upstream(&self, kind: UpstreamKind) -> UpstreamGuard {
        self.inner.upstream[kind.index()]
            .active
            .fetch_add(1, Ordering::Relaxed);
        UpstreamGuard {
            observability: self.clone(),
            kind,
            success: false,
        }
    }

    pub fn try_begin_cpu_profile(&self) -> Option<CpuProfileGuard> {
        self.inner
            .cpu_profile_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .ok()
            .map(|_| CpuProfileGuard(self.clone()))
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        let http = self.inner.http.lock().expect("HTTP metrics lock poisoned");
        RuntimeSnapshot {
            uptime: self.inner.started_at.elapsed(),
            active_http: self.inner.active_http.load(Ordering::Relaxed),
            request_buffer_bytes: self.inner.request_buffer_bytes.load(Ordering::Relaxed),
            active_sse: self.inner.active_sse.load(Ordering::Relaxed),
            sse_buffer_bytes: self.inner.sse_buffer_bytes.load(Ordering::Relaxed),
            cpu_profile_active: self.inner.cpu_profile_active.load(Ordering::Relaxed),
            http: http
                .iter()
                .map(|(key, value)| HttpSnapshot {
                    method: key.method,
                    route: key.route.clone(),
                    requests_by_status: value.requests_by_status.clone(),
                    duration_count: value.duration_count,
                    duration_sum: value.duration_sum,
                    duration_buckets: value.duration_buckets,
                    request_bytes_total: value.request_bytes_total,
                    response_bytes_total: value.response_bytes_total,
                })
                .collect(),
            upstream: UpstreamKind::ALL.map(|kind| UpstreamSnapshot {
                kind,
                active: self.inner.upstream[kind.index()]
                    .active
                    .load(Ordering::Relaxed),
                success: self.inner.upstream[kind.index()]
                    .success
                    .load(Ordering::Relaxed),
                error: self.inner.upstream[kind.index()]
                    .error
                    .load(Ordering::Relaxed),
            }),
            process: process_snapshot(),
            allocator: allocator_snapshot(),
        }
    }

    fn finish_http(
        &self,
        key: HttpKey,
        elapsed: Duration,
        status: &'static str,
        request_bytes: u64,
        response_bytes: u64,
    ) {
        self.inner.active_http.fetch_sub(1, Ordering::Relaxed);
        self.inner
            .request_buffer_bytes
            .fetch_sub(request_bytes, Ordering::Relaxed);
        let mut http = self.inner.http.lock().expect("HTTP metrics lock poisoned");
        let series = http.entry(key).or_default();
        *series.requests_by_status.entry(status).or_default() += 1;
        series.duration_count += 1;
        series.duration_sum += elapsed.as_secs_f64();
        for (index, upper_bound) in HTTP_DURATION_BUCKETS.iter().enumerate() {
            if elapsed.as_secs_f64() <= *upper_bound {
                series.duration_buckets[index] += 1;
            }
        }
        series.request_bytes_total = series.request_bytes_total.saturating_add(request_bytes);
        series.response_bytes_total = series.response_bytes_total.saturating_add(response_bytes);
    }
}

pub struct HttpRequestGuard {
    observability: Observability,
    key: HttpKey,
    started_at: Instant,
    request_bytes: u64,
    finished: bool,
}

impl HttpRequestGuard {
    pub fn finish(mut self, status: u16, response_bytes: u64) {
        self.finished = true;
        self.observability.finish_http(
            self.key.clone(),
            self.started_at.elapsed(),
            status_class(status),
            self.request_bytes,
            response_bytes,
        );
    }
}

impl Drop for HttpRequestGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.observability.finish_http(
                self.key.clone(),
                self.started_at.elapsed(),
                "aborted",
                self.request_bytes,
                0,
            );
        }
    }
}

pub struct StreamGuard {
    observability: Observability,
    buffer_bytes: u64,
}

impl StreamGuard {
    pub fn set_buffer_bytes(&mut self, bytes: u64) {
        if bytes >= self.buffer_bytes {
            self.observability
                .inner
                .sse_buffer_bytes
                .fetch_add(bytes - self.buffer_bytes, Ordering::Relaxed);
        } else {
            self.observability
                .inner
                .sse_buffer_bytes
                .fetch_sub(self.buffer_bytes - bytes, Ordering::Relaxed);
        }
        self.buffer_bytes = bytes;
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.observability
            .inner
            .sse_buffer_bytes
            .fetch_sub(self.buffer_bytes, Ordering::Relaxed);
        self.observability
            .inner
            .active_sse
            .fetch_sub(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum UpstreamKind {
    Kubernetes,
    Prometheus,
    Webhook,
}

impl UpstreamKind {
    pub const ALL: [Self; 3] = [Self::Kubernetes, Self::Prometheus, Self::Webhook];

    const fn index(self) -> usize {
        match self {
            Self::Kubernetes => 0,
            Self::Prometheus => 1,
            Self::Webhook => 2,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Kubernetes => "kubernetes",
            Self::Prometheus => "prometheus",
            Self::Webhook => "webhook",
        }
    }
}

pub struct UpstreamGuard {
    observability: Observability,
    kind: UpstreamKind,
    success: bool,
}

impl UpstreamGuard {
    pub fn success(mut self) {
        self.success = true;
    }
}

impl Drop for UpstreamGuard {
    fn drop(&mut self) {
        let counters = &self.observability.inner.upstream[self.kind.index()];
        counters.active.fetch_sub(1, Ordering::Relaxed);
        if self.success {
            counters.success.fetch_add(1, Ordering::Relaxed);
        } else {
            counters.error.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub struct CpuProfileGuard(Observability);

impl Drop for CpuProfileGuard {
    fn drop(&mut self) {
        self.0
            .inner
            .cpu_profile_active
            .store(false, Ordering::Release);
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeSnapshot {
    pub uptime: Duration,
    pub active_http: u64,
    pub request_buffer_bytes: u64,
    pub active_sse: u64,
    pub sse_buffer_bytes: u64,
    pub cpu_profile_active: bool,
    pub http: Vec<HttpSnapshot>,
    pub upstream: [UpstreamSnapshot; 3],
    pub process: ProcessSnapshot,
    pub allocator: Option<AllocatorSnapshot>,
}

#[derive(Clone, Debug)]
pub struct HttpSnapshot {
    pub method: &'static str,
    pub route: String,
    pub requests_by_status: BTreeMap<&'static str, u64>,
    pub duration_count: u64,
    pub duration_sum: f64,
    pub duration_buckets: [u64; HTTP_DURATION_BUCKETS.len()],
    pub request_bytes_total: u64,
    pub response_bytes_total: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct UpstreamSnapshot {
    pub kind: UpstreamKind,
    pub active: u64,
    pub success: u64,
    pub error: u64,
}

fn normalized_method(method: &str) -> &'static str {
    match method {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    }
}

fn status_class(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

#[cfg(test)]
mod tests;
