use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, Bucket>>>,
    limit: u32,
    window: Duration,
}

struct Bucket {
    started: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            limit,
            window,
        }
    }

    pub async fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().await;
        if buckets.len() > 20_000 {
            buckets.retain(|_, value| now.duration_since(value.started) < self.window);
        }
        let bucket = buckets.entry(key.to_owned()).or_insert(Bucket {
            started: now,
            count: 0,
        });
        if now.duration_since(bucket.started) >= self.window {
            bucket.started = now;
            bucket.count = 0;
        }
        if bucket.count >= self.limit {
            return false;
        }
        bucket.count += 1;
        true
    }
}
