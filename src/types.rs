use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Unified game information aggregated from one or more providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameInfo {
    // --- identity ----------------------------------------------------------
    pub ids: ProviderIds,
    /// Which provider sourced this entry (or `Merged` when combined).
    pub source: ProviderKind,
    /// Matching confidence vs the search query, 0.0–1.0.
    pub confidence: f64,

    // --- core metadata -----------------------------------------------------
    pub title: String,
    pub alternative_titles: Vec<String>,
    pub summary: Option<String>,
    pub storyline: Option<String>,
    pub slug: Option<String>,

    // --- classification ----------------------------------------------------
    pub genres: Vec<Genre>,
    pub themes: Vec<String>,
    pub keywords: Vec<String>,
    pub category: GameCategory,
    pub status: ReleaseStatus,
    /// Content type (relevant for DLsite which hosts non-game works too).
    pub content_type: ContentType,
    pub age_rating: AgeRating,

    // --- platform & capabilities ------------------------------------------
    pub platforms: Vec<Platform>,
    pub game_modes: Vec<GameMode>,
    pub player_perspectives: Vec<PlayerPerspective>,
    pub player_count: Option<PlayerCount>,
    pub languages: Vec<Language>,
    pub file_formats: Vec<String>,

    // --- dates ------------------------------------------------------------
    pub release_date: Option<NaiveDate>,
    pub platform_releases: Vec<PlatformRelease>,
    pub updated_at: Option<DateTime<Utc>>,

    // --- credits ----------------------------------------------------------
    pub developers: Vec<Company>,
    pub publishers: Vec<Company>,

    // --- ratings ----------------------------------------------------------
    pub ratings: Vec<Rating>,

    // --- media ------------------------------------------------------------
    pub cover: Option<Image>,
    pub screenshots: Vec<Image>,
    pub artworks: Vec<Image>,
    pub videos: Vec<Video>,
    pub websites: Vec<Website>,

    // --- commercial -------------------------------------------------------
    pub price: Option<Price>,
    pub download_count: Option<u64>,
    /// File size in bytes (DLsite).
    pub file_size: Option<u64>,

    // --- relations --------------------------------------------------------
    pub franchise: Option<String>,
    pub series: Vec<String>,
    pub game_engines: Vec<String>,
    pub similar_games: Vec<GameRef>,

    // --- extension --------------------------------------------------------
    /// Raw provider-specific fields not covered by the unified schema.
    pub extra: HashMap<String, serde_json::Value>,
}

impl GameInfo {
    /// Convenience: average rating across all rating sources.
    #[must_use]
    pub fn average_rating(&self) -> Option<f64> {
        if self.ratings.is_empty() {
            return None;
        }
        let sum: f64 = self.ratings.iter().map(|r| r.score).sum();
        #[allow(clippy::cast_precision_loss)]
        Some(sum / self.ratings.len() as f64)
    }

    /// Best available cover URL.
    #[must_use]
    pub fn cover_url(&self) -> Option<&str> {
        self.cover.as_ref().map(|img| img.url.as_str())
    }
}

// ---------------------------------------------------------------------------
// Provider identity
// ---------------------------------------------------------------------------

/// Provider-specific IDs for cross-referencing results.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ProviderIds {
    pub igdb: Option<u64>,
    pub thegamesdb: Option<u64>,
    pub steam: Option<u64>,
    pub gog: Option<String>,
    /// Extra external IDs keyed by system name (`"epic"`, `"itch"`, …).
    pub external: HashMap<String, String>,
}

/// Which data provider sourced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    Igdb,
    TheGamesDb,
    Steam,
    /// Indicates a result merged from multiple providers.
    Merged,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Igdb => f.write_str("IGDB"),
            Self::TheGamesDb => f.write_str("TheGamesDB"),
            Self::Steam => f.write_str("Steam"),
            Self::Merged => f.write_str("Merged"),
        }
    }
}

// ---------------------------------------------------------------------------
// Classification enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Genre {
    Action,
    Adventure,
    RolePlaying,
    Strategy,
    Simulation,
    Puzzle,
    Sports,
    Racing,
    Fighting,
    Shooter,
    Horror,
    VisualNovel,
    Platformer,
    Stealth,
    Survival,
    BeatEmUp,
    MusicRhythm,
    Pinball,
    Quiz,
    CardGame,
    BoardGame,
    Moba,
    TowerDefense,
    Roguelike,
    HackAndSlash,
    Sandbox,
    OpenWorld,
    Indie,
    Arcade,
    TacticalRpg,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    Pc,
    Mac,
    Linux,
    Android,
    Ios,
    Web,
    PlayStation(u8),
    Xbox(XboxGen),
    Nintendo(NintendoPlatform),
    Arcade,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum XboxGen {
    Original,
    Xbox360,
    XboxOne,
    SeriesXS,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NintendoPlatform {
    Nes,
    Snes,
    N64,
    GameCube,
    Wii,
    WiiU,
    Switch,
    Switch2,
    GameBoy,
    GameBoyColor,
    GameBoyAdvance,
    Ds,
    ThreeDs,
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GameMode {
    SinglePlayer,
    Multiplayer,
    CoOp,
    SplitScreen,
    BattleRoyale,
    Mmo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlayerPerspective {
    FirstPerson,
    ThirdPerson,
    BirdViewIsometric,
    SideScroller,
    Text,
    Auditory,
    VirtualReality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerCount {
    pub min: u32,
    pub max: u32,
    pub online_max: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ReleaseStatus {
    Released,
    Alpha,
    Beta,
    EarlyAccess,
    Offline,
    Cancelled,
    Rumoured,
    Delisted,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum GameCategory {
    MainGame,
    DlcAddon,
    Expansion,
    Bundle,
    StandaloneExpansion,
    Mod,
    Episode,
    Season,
    Remake,
    Remaster,
    ExpandedGame,
    Port,
    Fork,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ContentType {
    Game,
    Other,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AgeRating {
    AllAges,
    Teen,
    Mature,
    AdultsOnly,
    #[default]
    Unknown,
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub kind: ImageKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageKind {
    Cover,
    Screenshot,
    Artwork,
    Banner,
    Thumbnail,
    Background,
    ClearLogo,
    Boxart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Video {
    pub url: String,
    pub name: Option<String>,
    pub kind: VideoKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VideoKind {
    Trailer,
    Gameplay,
    Review,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Website {
    pub url: String,
    pub kind: WebsiteKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WebsiteKind {
    Official,
    Steam,
    Gog,
    EpicGames,
    Twitter,
    Facebook,
    Instagram,
    Reddit,
    Wikipedia,
    Youtube,
    Twitch,
    Discord,
    ItchIo,
    Humble,
    Other(String),
}

// ---------------------------------------------------------------------------
// Credits
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Company {
    pub name: String,
    pub role: CompanyRole,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompanyRole {
    Developer,
    Publisher,
    Porting,
    Supporting,
}

// ---------------------------------------------------------------------------
// Dates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformRelease {
    pub platform: Platform,
    pub date: Option<NaiveDate>,
    pub region: Option<Region>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Region {
    Worldwide,
    NorthAmerica,
    Europe,
    Japan,
    Australia,
    China,
    Korea,
    Asia,
    Brazil,
    Other,
}

// ---------------------------------------------------------------------------
// Ratings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    /// Normalized score on a 0–100 scale.
    pub score: f64,
    pub count: Option<u32>,
    pub source: ProviderKind,
    pub kind: RatingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RatingKind {
    User,
    Critic,
    Aggregated,
}

// ---------------------------------------------------------------------------
// Commercial
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub amount: f64,
    /// ISO 4217 currency code (`"JPY"`, `"USD"`, …).
    pub currency: String,
    pub sale_amount: Option<f64>,
    pub on_sale: bool,
}

// ---------------------------------------------------------------------------
// Language support
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Language {
    /// ISO 639-1 code (`"ja"`, `"en"`, …).
    pub code: String,
    pub name: String,
    pub kind: LanguageSupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LanguageSupport {
    FullAudio,
    Subtitles,
    Interface,
    Full,
}

// ---------------------------------------------------------------------------
// References to other games
// ---------------------------------------------------------------------------

/// Lightweight reference to a related game (avoids recursive `GameInfo` nesting).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameRef {
    pub title: String,
    pub ids: ProviderIds,
}
