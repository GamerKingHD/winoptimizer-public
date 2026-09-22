//! Simple in-memory cache with TTL for expensive read-only queries.
//! Uses JSON serialization as the storage format so any Serialize/Deserialize
//! type can be cached without generic type parameters in the store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct CacheEntry {
    json: String,
    created_at: Instant,
}

/// Thread-safe, TTL-based cache keyed by command name.
pub struct CacheStore {
    entries: Mutex<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

impl CacheStore {
    /// Create a new cache with the given TTL.
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Try to retrieve a cached value. Returns `None` if missing or expired.
    pub fn get<T: for<'a> Deserialize<'a>>(&self, key: &str) -> Option<T> {
        let entries = self.entries.lock().ok()?;
        let entry = entries.get(key)?;
        if entry.created_at.elapsed() > self.ttl {
            return None;
        }
        serde_json::from_str(&entry.json).ok()
    }

    /// Store a value in the cache.
    pub fn set<T: Serialize>(&self, key: String, value: &T) {
        if let Ok(json) = serde_json::to_string(value) {
            if let Ok(mut entries) = self.entries.lock() {
                entries.insert(
                    key,
                    CacheEntry {
                        json,
                        created_at: Instant::now(),
                    },
                );
            }
        }
    }

    /// Invalidate a specific cache entry.
    pub fn invalidate(&self, key: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(key);
        }
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }
}
