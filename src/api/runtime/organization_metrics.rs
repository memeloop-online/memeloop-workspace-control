use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::Url;
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

use crate::observability::Observability;

mod collector;
mod query;

const CACHE_CAPACITY: usize = 128;
const CACHE_TTL: Duration = Duration::from_secs(20);
const MAX_COLLECTIONS: usize = 8;
const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default)]
pub(crate) struct OrganizationMetrics {
    pub(crate) cpu_millis: Option<u64>,
    pub(crate) memory_mib: Option<u64>,
    pub(crate) disk_bytes: Option<u64>,
    pub(crate) observed_at: Option<i64>,
    pub(crate) template_labels_complete: bool,
}

#[derive(Clone)]
pub(crate) struct OrganizationMetricsCache(Arc<CacheState>);

struct CacheState {
    entries: Mutex<HashMap<CacheKey, Arc<CacheEntry>>>,
    collections: Semaphore,
}

#[derive(Default)]
struct CacheEntry {
    observation: Mutex<Option<(Instant, OrganizationMetrics)>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    base_url: Option<Url>,
    installation_id: String,
    organization_id: Uuid,
    templates: Option<Vec<Uuid>>,
    active: u64,
    pvcs: u64,
}

impl Default for OrganizationMetricsCache {
    fn default() -> Self {
        Self(Arc::new(CacheState {
            entries: Mutex::new(HashMap::new()),
            collections: Semaphore::new(MAX_COLLECTIONS),
        }))
    }
}

impl OrganizationMetricsCache {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn fetch(
        &self,
        base_url: Option<&Url>,
        installation_id: &str,
        organization_id: Uuid,
        allowed_template_ids: Option<&[Uuid]>,
        expected_active: u64,
        expected_pvcs: u64,
        observability: &Observability,
    ) -> OrganizationMetrics {
        let mut templates = allowed_template_ids.map(<[Uuid]>::to_vec);
        if let Some(ids) = templates.as_mut() {
            ids.sort_unstable();
            ids.dedup();
        }
        let key = CacheKey {
            base_url: base_url.cloned(),
            installation_id: installation_id.to_owned(),
            organization_id,
            templates,
            active: expected_active,
            pvcs: expected_pvcs,
        };
        let Some(entry) = self.entry(&key).await else {
            return OrganizationMetrics::default();
        };
        let Ok(mut observation) =
            tokio::time::timeout(WAIT_TIMEOUT, entry.observation.lock()).await
        else {
            return OrganizationMetrics::default();
        };
        if let Some((at, value)) = observation.as_ref()
            && at.elapsed() < CACHE_TTL
        {
            return value.clone();
        }
        let Ok(Ok(_permit)) =
            tokio::time::timeout(WAIT_TIMEOUT, self.0.collections.acquire()).await
        else {
            return OrganizationMetrics::default();
        };
        let value = collector::collect(
            key.base_url.as_ref(),
            &key.installation_id,
            key.organization_id,
            key.templates.as_deref(),
            key.active,
            key.pvcs,
            observability,
        )
        .await;
        *observation = Some((Instant::now(), value.clone()));
        value
    }

    async fn entry(&self, key: &CacheKey) -> Option<Arc<CacheEntry>> {
        let mut entries = self.0.entries.lock().await;
        if let Some(entry) = entries.get(key) {
            return Some(entry.clone());
        }
        if entries.len() >= CACHE_CAPACITY {
            // An entry held by a collector or waiter cannot be evicted: doing so
            // would create a second concurrent collection for the same key.
            let idle = entries
                .iter()
                .find(|(_, entry)| Arc::strong_count(entry) == 1)
                .map(|(key, _)| key.clone());
            entries.remove(&idle?);
        }
        let entry = Arc::new(CacheEntry::default());
        entries.insert(key.clone(), entry.clone());
        Some(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(organization_id: Uuid) -> CacheKey {
        CacheKey {
            base_url: None,
            installation_id: "test".to_owned(),
            organization_id,
            templates: None,
            active: 1,
            pvcs: 1,
        }
    }

    #[tokio::test]
    async fn cache_is_bounded_and_retains_in_flight_identity() {
        let cache = OrganizationMetricsCache::default();
        let active_key = key(Uuid::now_v7());
        let active = cache.entry(&active_key).await.unwrap();
        for _ in 0..CACHE_CAPACITY * 2 {
            let _ = cache.entry(&key(Uuid::now_v7())).await.unwrap();
        }
        assert_eq!(cache.0.entries.lock().await.len(), CACHE_CAPACITY);
        assert!(Arc::ptr_eq(
            &active,
            &cache.entry(&active_key).await.unwrap()
        ));
    }

    #[tokio::test]
    async fn saturation_does_not_create_unbounded_in_flight_entries() {
        let cache = OrganizationMetricsCache::default();
        let mut held = Vec::new();
        for _ in 0..CACHE_CAPACITY {
            held.push(cache.entry(&key(Uuid::now_v7())).await.unwrap());
        }
        assert!(cache.entry(&key(Uuid::now_v7())).await.is_none());
        assert_eq!(cache.0.entries.lock().await.len(), CACHE_CAPACITY);
    }

    #[tokio::test]
    async fn clones_share_entries_but_separate_instances_do_not() {
        let cache = OrganizationMetricsCache::default();
        let scope = key(Uuid::now_v7());
        let entry = cache.entry(&scope).await.unwrap();
        assert!(Arc::ptr_eq(
            &entry,
            &cache.clone().entry(&scope).await.unwrap()
        ));
        assert!(!Arc::ptr_eq(
            &entry,
            &OrganizationMetricsCache::default()
                .entry(&scope)
                .await
                .unwrap()
        ));
    }
}
