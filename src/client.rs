use crate::{
    error::Error,
    matcher::{MatcherConfig, SmartMatcher},
    provider::{GameProvider, ProviderResult},
    query::SearchQuery,
    types::{GameInfo, ProviderKind},
};

/// Main entry point for multi-provider game metadata retrieval.
///
/// Build with [`GameInfoClient::builder()`].
pub struct GameInfoClient {
    providers: Vec<Box<dyn GameProvider>>,
    matcher: SmartMatcher,
}

impl GameInfoClient {
    #[must_use]
    pub fn builder() -> GameInfoClientBuilder {
        GameInfoClientBuilder::default()
    }

    /// Search all configured providers, rank and deduplicate results.
    ///
    /// Partial failures (individual provider errors) are logged as warnings;
    /// [`Error::AllProvidersFailed`] is returned only when every provider fails.
    ///
    /// # Errors
    /// - [`Error::NoProviders`] – no providers configured.
    /// - [`Error::AllProvidersFailed`] – every provider returned an error.
    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<GameInfo>, Error> {
        if self.providers.is_empty() {
            return Err(Error::NoProviders);
        }

        let mut all: Vec<ProviderResult> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        for provider in &self.providers {
            match provider.search(query).await {
                Ok(mut r) => all.append(&mut r),
                Err(e) => {
                    tracing::warn!(provider = %provider.kind(), error = %e, "provider search failed");
                    errors.push(format!("{}: {e}", provider.kind()));
                }
            }
        }

        if all.is_empty() && !errors.is_empty() {
            return Err(Error::AllProvidersFailed {
                details: errors.join("; "),
            });
        }

        let ranked = self.matcher.rank(&query.title, all);
        Ok(self.matcher.dedup_merge(ranked))
    }

    /// Search a single provider by kind, returning ranked results.
    ///
    /// # Errors
    /// - [`Error::ProviderNotFound`] – provider not in client.
    /// - Propagates the provider's own errors.
    pub async fn search_provider(
        &self,
        kind: ProviderKind,
        query: &SearchQuery,
    ) -> Result<Vec<GameInfo>, Error> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.kind() == kind)
            .ok_or_else(|| Error::ProviderNotFound {
                provider: kind.to_string(),
            })?;

        let results = provider.search(query).await?;
        Ok(self.matcher.rank(&query.title, results))
    }

    /// Fetch a specific game by ID from `kind`.
    ///
    /// # Errors
    /// - [`Error::ProviderNotFound`] – provider not in client.
    /// - Propagates the provider's own errors.
    pub async fn fetch_by_id(
        &self,
        kind: ProviderKind,
        id: &str,
    ) -> Result<Option<GameInfo>, Error> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.kind() == kind)
            .ok_or_else(|| Error::ProviderNotFound {
                provider: kind.to_string(),
            })?;

        provider.fetch_by_id(id).await
    }
}

/// Builder for [`GameInfoClient`].
#[derive(Default)]
pub struct GameInfoClientBuilder {
    providers: Vec<Box<dyn GameProvider>>,
    matcher_config: Option<MatcherConfig>,
    #[cfg(feature = "cache")]
    cache_config: Option<crate::cache::CacheConfig>,
}

impl GameInfoClientBuilder {
    /// Add a provider or a config struct that converts into one.
    ///
    /// Accepts anything that implements `Into<Box<dyn GameProvider>>`:
    /// - A provider directly (`IgdbProvider::new(cfg)`)
    /// - A bare config struct (`IgdbConfig { … }`)
    #[must_use]
    pub fn provider(mut self, p: impl Into<Box<dyn GameProvider>>) -> Self {
        self.providers.push(p.into());
        self
    }

    #[must_use]
    pub fn matcher_config(mut self, config: MatcherConfig) -> Self {
        self.matcher_config = Some(config);
        self
    }

    /// Enable transparent disk caching for all providers.
    ///
    /// Each provider's responses are cached on disk under `config.dir`.
    /// Pass [`CacheConfig::default()`](crate::cache::CacheConfig::default) to use the
    /// OS-standard cache directory with sensible TTLs (24 h for searches, 7 d for by-ID).
    #[cfg(feature = "cache")]
    #[must_use]
    pub fn with_cache(mut self, config: crate::cache::CacheConfig) -> Self {
        self.cache_config = Some(config);
        self
    }

    #[must_use]
    pub fn build(self) -> GameInfoClient {
        let matcher_config = self.matcher_config;
        let providers = self.providers;
        #[cfg(feature = "cache")]
        let cache_config = self.cache_config;

        let matcher = SmartMatcher::new(matcher_config.unwrap_or_default());

        #[cfg(feature = "cache")]
        let providers = apply_cache(providers, cache_config);

        GameInfoClient { providers, matcher }
    }
}

#[cfg(feature = "cache")]
fn apply_cache(
    providers: Vec<Box<dyn GameProvider>>,
    config: Option<crate::cache::CacheConfig>,
) -> Vec<Box<dyn GameProvider>> {
    use crate::cache::{CachedProvider, DiskCache};
    let Some(cfg) = config else {
        return providers;
    };
    let cache = DiskCache::new(cfg);
    providers
        .into_iter()
        .map(|p| -> Box<dyn GameProvider> { Box::new(CachedProvider::new(p, cache.clone())) })
        .collect()
}
