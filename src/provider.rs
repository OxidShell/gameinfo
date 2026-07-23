#![allow(clippy::module_name_repetitions)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::Error,
    query::SearchQuery,
    types::{GameInfo, ProviderKind},
};

/// Raw result from a single provider before ranking/merging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResult {
    pub info: GameInfo,
    /// Provider's own relevance signal, normalised to 0.0–1.0.
    pub raw_score: f64,
    pub provider: ProviderKind,
}

/// A source of game metadata.
#[async_trait]
pub trait GameProvider: Send + Sync {
    fn kind(&self) -> ProviderKind;

    /// Search for games matching `query`.
    ///
    /// # Errors
    /// Returns [`Error`] on network, auth, or parse failures.
    async fn search(&self, query: &SearchQuery) -> Result<Vec<ProviderResult>, Error>;

    /// Fetch a specific game by provider-native ID string.
    ///
    /// # Errors
    /// Returns [`Error`] on network, auth, or parse failures.
    async fn fetch_by_id(&self, id: &str) -> Result<Option<GameInfo>, Error>;
}

/// Any `GameProvider` value can be boxed into a trait object.
///
/// This makes `.provider(my_provider)` work in the builder without an explicit `Box::new`.
impl<P: GameProvider + 'static> From<P> for Box<dyn GameProvider> {
    fn from(p: P) -> Self {
        Box::new(p)
    }
}
