#![allow(clippy::module_name_repetitions)]

//! Disk-backed cache for provider results.
//!
//! Enabled via the `cache` feature. Add `.with_cache(CacheConfig::default())` to the
//! [`GameInfoClientBuilder`](crate::client::GameInfoClientBuilder) to transparently cache
//! all provider responses on disk.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{
    error::Error,
    provider::{GameProvider, ProviderResult},
    query::SearchQuery,
    types::{GameInfo, ProviderKind},
};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the disk cache.
pub struct CacheConfig {
    /// Root directory for cached files. Defaults to the OS cache directory.
    pub dir: PathBuf,
    /// TTL for `search` results. Defaults to 24 hours.
    pub ttl_search: Duration,
    /// TTL for `fetch_by_id` results. Defaults to 7 days.
    pub ttl_by_id: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        #[allow(clippy::duration_suboptimal_units)] // from_days/from_weeks not stable yet
        Self {
            dir: default_cache_dir(),
            ttl_search: Duration::from_secs(86_400),
            ttl_by_id: Duration::from_secs(604_800),
        }
    }
}

// ---------------------------------------------------------------------------
// DiskCache
// ---------------------------------------------------------------------------

/// Cloneable handle to the shared disk cache. Cheap to clone (backed by [`Arc`]).
#[derive(Clone)]
pub struct DiskCache {
    root: Arc<PathBuf>,
    ttl_search: Duration,
    ttl_by_id: Duration,
}

impl DiskCache {
    #[must_use]
    pub fn new(config: CacheConfig) -> Self {
        Self {
            root: Arc::new(config.dir),
            ttl_search: config.ttl_search,
            ttl_by_id: config.ttl_by_id,
        }
    }

    fn entry_path(&self, provider: ProviderKind, ns: &str, key: &str) -> PathBuf {
        self.root
            .join(provider.to_string().to_ascii_lowercase())
            .join(ns)
            .join(format!("{key}.json"))
    }

    async fn read<T: for<'de> Deserialize<'de>>(&self, path: &Path, ttl: Duration) -> Option<T> {
        let content = tokio::fs::read_to_string(path).await.ok()?;
        let entry: CacheEntry<T> = serde_json::from_str(&content).ok()?;
        let age_secs = chrono::Utc::now().timestamp() - entry.cached_at;
        let ttl_i64 = i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX);
        if age_secs > ttl_i64 {
            return None;
        }
        Some(entry.data)
    }

    async fn write<T: Serialize + ?Sized>(&self, path: &Path, data: &T) {
        let entry = CacheEntry {
            cached_at: chrono::Utc::now().timestamp(),
            data,
        };
        let Ok(json) = serde_json::to_string(&entry) else {
            return;
        };
        if let Some(parent) = path.parent()
            && tokio::fs::create_dir_all(parent).await.is_err()
        {
            return;
        }
        if let Err(e) = tokio::fs::write(path, &json).await {
            warn!(path = %path.display(), error = %e, "cache write failed");
        }
    }

    pub(crate) async fn get_search(
        &self,
        provider: ProviderKind,
        query: &SearchQuery,
    ) -> Option<Vec<ProviderResult>> {
        let path = self.entry_path(provider, "search", &search_key(query));
        self.read(&path, self.ttl_search).await
    }

    pub(crate) async fn set_search(
        &self,
        provider: ProviderKind,
        query: &SearchQuery,
        results: &[ProviderResult],
    ) {
        let path = self.entry_path(provider, "search", &search_key(query));
        self.write(&path, results).await;
    }

    pub(crate) async fn get_by_id(&self, provider: ProviderKind, id: &str) -> Option<GameInfo> {
        let path = self.entry_path(provider, "id", &sanitize_key(id));
        self.read(&path, self.ttl_by_id).await
    }

    pub(crate) async fn set_by_id(&self, provider: ProviderKind, id: &str, info: &GameInfo) {
        let path = self.entry_path(provider, "id", &sanitize_key(id));
        self.write(&path, info).await;
    }
}

// ---------------------------------------------------------------------------
// CachedProvider
// ---------------------------------------------------------------------------

/// Wraps any [`GameProvider`] with a transparent disk-cache layer.
///
/// Constructed automatically by
/// [`GameInfoClientBuilder::with_cache`](crate::client::GameInfoClientBuilder::with_cache).
pub struct CachedProvider {
    inner: Box<dyn GameProvider>,
    cache: DiskCache,
}

impl CachedProvider {
    #[must_use]
    pub fn new(inner: Box<dyn GameProvider>, cache: DiskCache) -> Self {
        Self { inner, cache }
    }
}

#[async_trait]
impl GameProvider for CachedProvider {
    fn kind(&self) -> ProviderKind {
        self.inner.kind()
    }

    async fn search(&self, query: &SearchQuery) -> Result<Vec<ProviderResult>, Error> {
        let kind = self.inner.kind();
        if let Some(hit) = self.cache.get_search(kind, query).await {
            return Ok(hit);
        }
        let results = self.inner.search(query).await?;
        self.cache.set_search(kind, query, &results).await;
        Ok(results)
    }

    async fn fetch_by_id(&self, id: &str) -> Result<Option<GameInfo>, Error> {
        let kind = self.inner.kind();
        if let Some(hit) = self.cache.get_by_id(kind, id).await {
            return Ok(Some(hit));
        }
        let result = self.inner.fetch_by_id(id).await?;
        if let Some(ref info) = result {
            self.cache.set_by_id(kind, id, info).await;
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct CacheEntry<T> {
    cached_at: i64,
    data: T,
}

fn search_key(query: &SearchQuery) -> String {
    let mut h = DefaultHasher::new();
    query.title.hash(&mut h);
    query.platform.hash(&mut h);
    query.year.hash(&mut h);
    query.limit.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn sanitize_key(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    out.truncate(64);
    out
}

fn default_cache_dir() -> PathBuf {
    default_cache_dir_inner().unwrap_or_else(|| PathBuf::from(".gameinfo_cache"))
}

fn default_cache_dir_inner() -> Option<PathBuf> {
    // XDG_CACHE_HOME takes precedence on any Unix-like system
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg).join("gameinfo"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(platform_subdir(&home))
}

#[cfg(target_os = "macos")]
fn platform_subdir(home: &Path) -> PathBuf {
    home.join("Library/Caches/gameinfo")
}

#[cfg(target_os = "windows")]
fn platform_subdir(_home: &Path) -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("gameinfo\\cache"))
        .unwrap_or_else(|| PathBuf::from(".gameinfo_cache"))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_subdir(home: &Path) -> PathBuf {
    home.join(".cache/gameinfo")
}
