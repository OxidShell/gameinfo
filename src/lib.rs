#![allow(clippy::doc_markdown)] // IGDB, TheGamesDB are proper nouns, not code identifiers

//! # gameinfo
//!
//! Unified game metadata library. Queries IGDB, TheGamesDB and Steam in parallel,
//! then uses a smart-matching algorithm to rank, deduplicate and optionally merge
//! results into a single [`GameInfo`] per title.
//!
//! ## Quick start
//!
//! ```no_run
//! use gameinfo::{
//!     GameInfoClient, SearchQuery,
//!     providers::{IgdbConfig, IgdbProvider, TheGamesDbConfig, TheGamesDbProvider},
//! };
//!
//! #[tokio::main]
//! async fn main() -> Result<(), gameinfo::Error> {
//!     let client = GameInfoClient::builder()
//!         .provider(IgdbProvider::new(IgdbConfig {
//!             client_id:     std::env::var("IGDB_CLIENT_ID").unwrap(),
//!             client_secret: std::env::var("IGDB_CLIENT_SECRET").unwrap(),
//!         }))
//!         .provider(TheGamesDbProvider::new(TheGamesDbConfig {
//!             api_key: std::env::var("TGDB_API_KEY").unwrap(),
//!         }))
//!         .build();
//!
//!     let results = client.search(&SearchQuery::new("Hollow Knight")).await?;
//!
//!     for game in &results {
//!         println!(
//!             "[{:.0}%] {} — {}",
//!             game.confidence * 100.0,
//!             game.title,
//!             game.average_rating().map_or("n/a".into(), |r| format!("{r:.1}/100")),
//!         );
//!     }
//!
//!     Ok(())
//! }
//! ```

pub mod client;
pub mod error;
pub mod matcher;
pub mod provider;
pub mod providers;
pub mod query;
#[cfg(feature = "steam")]
pub mod steam;
pub mod types;

#[cfg(feature = "cache")]
pub mod cache;

mod normalize;

#[cfg(feature = "cache")]
pub use cache::CacheConfig;
pub use client::{GameInfoClient, GameInfoClientBuilder};
pub use error::Error;
pub use matcher::MatcherConfig;
pub use provider::{GameProvider, ProviderResult};
pub use query::SearchQuery;
#[cfg(feature = "steam")]
pub use steam::{InstalledGame, SteamLibrary};
pub use types::{
    AgeRating, Company, CompanyRole, ContentType, GameCategory, GameInfo, GameMode, GameRef, Genre,
    Image, ImageKind, Language, LanguageSupport, NintendoPlatform, Platform, PlatformRelease,
    PlayerCount, PlayerPerspective, Price, ProviderIds, ProviderKind, Rating, RatingKind, Region,
    ReleaseStatus, Video, VideoKind, Website, WebsiteKind, XboxGen,
};
