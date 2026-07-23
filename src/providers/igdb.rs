use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{debug, instrument};

use crate::{
    error::Error,
    provider::{GameProvider, ProviderResult},
    query::SearchQuery,
    types::{
        AgeRating, Company, CompanyRole, ContentType, GameCategory, GameInfo, GameMode, GameRef,
        Genre, Image, ImageKind, NintendoPlatform, Platform, PlatformRelease, PlayerPerspective,
        ProviderIds, ProviderKind, Rating, RatingKind, Region, ReleaseStatus, Video, VideoKind,
        Website, WebsiteKind, XboxGen,
    },
};

const IGDB_API: &str = "https://api.igdb.com/v4";
const TWITCH_TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";

/// IGDB / Twitch credentials.
///
/// Obtain at <https://dev.twitch.tv/console>.
pub struct IgdbConfig {
    pub client_id: String,
    pub client_secret: String,
}

struct TokenCache {
    token: String,
    expires_at: Instant,
}

pub struct IgdbProvider {
    client: reqwest::Client,
    config: IgdbConfig,
    cache: RwLock<Option<TokenCache>>,
}

impl From<IgdbConfig> for IgdbProvider {
    fn from(config: IgdbConfig) -> Self {
        Self::new(config)
    }
}

impl From<IgdbConfig> for Box<dyn crate::provider::GameProvider> {
    fn from(config: IgdbConfig) -> Self {
        Box::new(IgdbProvider::from(config))
    }
}

impl IgdbProvider {
    #[must_use]
    pub fn new(config: IgdbConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            cache: RwLock::new(None),
        }
    }

    async fn token(&self) -> Result<String, Error> {
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            expires_in: u64,
        }

        // 60-second buffer to avoid racing with expiry at the exact boundary
        #[allow(clippy::duration_suboptimal_units)] // Duration::from_mins requires Rust ≥ 1.87
        let buffer = Duration::from_secs(60);

        // Fast path: valid cached token
        {
            let guard = self.cache.read().await;
            if let Some(ref c) = *guard
                && c.expires_at > Instant::now() + buffer
            {
                return Ok(c.token.clone());
            }
        }

        // Slow path: refresh (double-check after acquiring write lock)
        let mut guard = self.cache.write().await;
        if let Some(ref c) = *guard
            && c.expires_at > Instant::now() + buffer
        {
            return Ok(c.token.clone());
        }

        let resp = self
            .client
            .post(TWITCH_TOKEN_URL)
            .query(&[
                ("client_id", &self.config.client_id),
                ("client_secret", &self.config.client_secret),
                ("grant_type", &"client_credentials".to_string()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(Error::Auth {
                provider: "IGDB".into(),
                reason: format!("Twitch token endpoint returned {}", resp.status()),
            });
        }

        let tok: TokenResponse = resp.json().await?;
        let expires_at = Instant::now() + Duration::from_secs(tok.expires_in);
        *guard = Some(TokenCache {
            token: tok.access_token.clone(),
            expires_at,
        });
        Ok(tok.access_token)
    }

    async fn post_apicalypse(
        &self,
        endpoint: &str,
        body: &str,
    ) -> Result<serde_json::Value, Error> {
        let token = self.token().await?;
        let url = format!("{IGDB_API}/{endpoint}");
        debug!(url, "IGDB request");

        let resp = self
            .client
            .post(&url)
            .header("Client-ID", &self.config.client_id)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "text/plain")
            .body(body.to_string())
            .send()
            .await?;

        if resp.status().as_u16() == 429 {
            return Err(Error::RateLimit {
                provider: "IGDB".into(),
            });
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let msg = resp.text().await.unwrap_or_default();
            return Err(Error::Provider {
                provider: "IGDB".into(),
                message: format!("{status}: {msg}"),
            });
        }

        resp.json::<serde_json::Value>().await.map_err(Error::Http)
    }

    fn fields() -> &'static str {
        "fields \
            id,name,slug,summary,storyline,\
            alternative_names.name,\
            genres.id,genres.name,\
            themes.name,\
            keywords.name,\
            platforms.id,platforms.name,\
            cover.url,cover.width,cover.height,\
            screenshots.url,screenshots.width,screenshots.height,\
            artworks.url,artworks.width,artworks.height,\
            videos.video_id,videos.name,\
            websites.url,websites.category,\
            first_release_date,\
            release_dates.date,release_dates.platform.name,release_dates.region,\
            rating,rating_count,\
            aggregated_rating,aggregated_rating_count,\
            involved_companies.company.name,involved_companies.company.url,\
            involved_companies.developer,involved_companies.publisher,involved_companies.supporting,\
            game_modes.id,\
            player_perspectives.id,\
            franchise.name,\
            franchises.name,\
            collection.name,\
            game_engines.name,\
            status,category,\
            age_ratings.rating,age_ratings.category,\
            similar_games.id,similar_games.name,\
            updated_at;\
        "
    }

    fn map_results(json: &serde_json::Value, limit: usize) -> Vec<ProviderResult> {
        let Some(arr) = json.as_array() else {
            return Vec::new();
        };
        arr.iter().take(limit).filter_map(Self::map_one).collect()
    }

    #[allow(clippy::too_many_lines)]
    fn map_one(v: &serde_json::Value) -> Option<ProviderResult> {
        let title = v["name"].as_str()?.to_string();

        let ids = ProviderIds {
            igdb: v["id"].as_u64(),
            ..Default::default()
        };

        let alternative_titles = v["alternative_names"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|n| n["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let genres = v["genres"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|g| g["id"].as_u64().map(igdb_genre))
                    .collect()
            })
            .unwrap_or_default();

        let themes = v["themes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let keywords = v["keywords"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|k| k["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let platforms = v["platforms"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p["id"].as_u64().map(igdb_platform))
                    .collect()
            })
            .unwrap_or_default();

        let cover = map_igdb_image(&v["cover"], ImageKind::Cover);

        let screenshots = v["screenshots"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| map_igdb_image(s, ImageKind::Screenshot))
                    .collect()
            })
            .unwrap_or_default();

        let artworks = v["artworks"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| map_igdb_image(a, ImageKind::Artwork))
                    .collect()
            })
            .unwrap_or_default();

        let videos = v["videos"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|vid| {
                        vid["video_id"].as_str().map(|yt_id| Video {
                            url: format!("https://www.youtube.com/watch?v={yt_id}"),
                            name: vid["name"].as_str().map(String::from),
                            kind: VideoKind::Trailer,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let websites = v["websites"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|w| {
                        w["url"].as_str().map(|url| Website {
                            url: url.to_string(),
                            kind: igdb_website_category(w["category"].as_u64().unwrap_or(0)),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let release_date = v["first_release_date"].as_i64().and_then(timestamp_to_date);

        let platform_releases = v["release_dates"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|rd| {
                        let platform_name = rd["platform"]["name"].as_str()?;
                        Some(PlatformRelease {
                            platform: platform_from_name(platform_name),
                            date: rd["date"].as_i64().and_then(timestamp_to_date),
                            region: rd["region"].as_u64().map(igdb_region),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut developers: Vec<Company> = Vec::new();
        let mut publishers: Vec<Company> = Vec::new();
        if let Some(arr) = v["involved_companies"].as_array() {
            for ic in arr {
                let name = match ic["company"]["name"].as_str() {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let url = ic["company"]["url"].as_str().map(String::from);
                if ic["developer"].as_bool().unwrap_or(false) {
                    developers.push(Company {
                        name: name.clone(),
                        role: CompanyRole::Developer,
                        url: url.clone(),
                    });
                }
                if ic["publisher"].as_bool().unwrap_or(false) {
                    publishers.push(Company {
                        name: name.clone(),
                        role: CompanyRole::Publisher,
                        url: url.clone(),
                    });
                }
                if ic["supporting"].as_bool().unwrap_or(false) {
                    developers.push(Company {
                        name,
                        role: CompanyRole::Supporting,
                        url,
                    });
                }
            }
        }

        let mut ratings: Vec<Rating> = Vec::new();
        if let Some(score) = v["rating"].as_f64() {
            ratings.push(Rating {
                score,
                count: v["rating_count"].as_u64().map(u64_to_u32),
                source: ProviderKind::Igdb,
                kind: RatingKind::User,
            });
        }
        if let Some(score) = v["aggregated_rating"].as_f64() {
            ratings.push(Rating {
                score,
                count: v["aggregated_rating_count"].as_u64().map(u64_to_u32),
                source: ProviderKind::Igdb,
                kind: RatingKind::Critic,
            });
        }

        let game_modes = v["game_modes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_u64().map(igdb_game_mode))
                    .collect()
            })
            .unwrap_or_default();

        let player_perspectives = v["player_perspectives"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p["id"].as_u64().map(igdb_perspective))
                    .collect()
            })
            .unwrap_or_default();

        let franchise = v["franchise"]["name"]
            .as_str()
            .or_else(|| v["collection"]["name"].as_str())
            .map(String::from);

        let series: Vec<String> = v["franchises"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| f["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let game_engines: Vec<String> = v["game_engines"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let similar_games = v["similar_games"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|sg| {
                        sg["name"].as_str().map(|name| GameRef {
                            title: name.to_string(),
                            ids: ProviderIds {
                                igdb: sg["id"].as_u64(),
                                ..Default::default()
                            },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let age_rating = igdb_age_rating(&v["age_ratings"]);
        let status = igdb_status(v["status"].as_u64());
        let category = igdb_category(v["category"].as_u64());

        let updated_at = v["updated_at"]
            .as_i64()
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0));

        let raw_score = v["rating"].as_f64().unwrap_or(50.0) / 100.0;

        Some(ProviderResult {
            raw_score,
            provider: ProviderKind::Igdb,
            info: GameInfo {
                ids,
                source: ProviderKind::Igdb,
                confidence: 0.0,
                title,
                alternative_titles,
                summary: v["summary"].as_str().map(String::from),
                storyline: v["storyline"].as_str().map(String::from),
                slug: v["slug"].as_str().map(String::from),
                genres,
                themes,
                keywords,
                category,
                status,
                content_type: ContentType::Game,
                age_rating,
                platforms,
                game_modes,
                player_perspectives,
                player_count: None,
                languages: Vec::new(),
                file_formats: Vec::new(),
                release_date,
                platform_releases,
                updated_at,
                developers,
                publishers,
                ratings,
                cover,
                screenshots,
                artworks,
                videos,
                websites,
                price: None,
                download_count: None,
                file_size: None,
                franchise,
                series,
                game_engines,
                similar_games,
                extra: HashMap::new(),
            },
        })
    }
}

#[async_trait]
impl GameProvider for IgdbProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Igdb
    }

    #[instrument(skip(self), fields(provider = "IGDB"))]
    async fn search(&self, query: &SearchQuery) -> Result<Vec<ProviderResult>, Error> {
        let limit = query.limit.min(500);
        let escaped = query.title.replace('"', "\\\"");
        let body = format!(
            "{fields}\
            where name ~ *\"{escaped}\"*;\
            limit {limit};\
            sort rating desc;",
            fields = Self::fields(),
        );
        let json = self.post_apicalypse("games", &body).await?;
        Ok(Self::map_results(&json, limit as usize))
    }

    #[instrument(skip(self), fields(provider = "IGDB", id))]
    async fn fetch_by_id(&self, id: &str) -> Result<Option<GameInfo>, Error> {
        let parsed: u64 = id
            .parse()
            .map_err(|_| Error::NotFound { id: id.to_string() })?;
        let body = format!(
            "{fields}\
            where id = {parsed};",
            fields = Self::fields(),
        );
        let json = self.post_apicalypse("games", &body).await?;
        Ok(Self::map_results(&json, 1)
            .into_iter()
            .next()
            .map(|r| r.info))
    }
}

// ---------------------------------------------------------------------------
// Mapping helpers
// ---------------------------------------------------------------------------

fn timestamp_to_date(ts: i64) -> Option<chrono::NaiveDate> {
    chrono::DateTime::from_timestamp(ts, 0).map(|dt: chrono::DateTime<chrono::Utc>| dt.date_naive())
}

fn igdb_image_url(raw: &str) -> String {
    let with_scheme = if raw.starts_with("//") {
        format!("https:{raw}")
    } else {
        raw.to_string()
    };
    // Upgrade thumbnail to cover_big for better resolution
    with_scheme.replace("/t_thumb/", "/t_cover_big/")
}

fn map_igdb_image(v: &serde_json::Value, kind: ImageKind) -> Option<Image> {
    v["url"].as_str().map(|url| Image {
        url: igdb_image_url(url),
        width: v["width"].as_u64().map(u64_to_u32),
        height: v["height"].as_u64().map(u64_to_u32),
        kind,
    })
}

fn igdb_genre(id: u64) -> Genre {
    match id {
        4 => Genre::Fighting,
        5 => Genre::Shooter,
        7 => Genre::MusicRhythm,
        8 => Genre::Platformer,
        9 => Genre::Puzzle,
        10 => Genre::Racing,
        11 | 15 | 16 | 24 => Genre::Strategy,
        12 => Genre::RolePlaying,
        13 => Genre::Simulation,
        14 => Genre::Sports,
        25 => Genre::HackAndSlash,
        26 => Genre::Quiz,
        30 => Genre::Pinball,
        31 => Genre::Adventure,
        32 => Genre::Indie,
        33 => Genre::Arcade,
        34 => Genre::VisualNovel,
        35 => Genre::CardGame,
        36 => Genre::Moba,
        _ => Genre::Other(format!("igdb:{id}")),
    }
}

/// Saturating cast: values above `u32::MAX` are clamped rather than wrapping.
fn u64_to_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn igdb_platform(id: u64) -> Platform {
    match id {
        6 => Platform::Pc,
        14 => Platform::Mac,
        3 => Platform::Linux,
        34 => Platform::Android,
        39 => Platform::Ios,
        48 => Platform::PlayStation(4),
        167 => Platform::PlayStation(5),
        7 | 46 => Platform::PlayStation(1),
        8 => Platform::PlayStation(3),
        9 => Platform::PlayStation(2),
        // PS Vita mapped as a distinct platform would need its own variant;
        // treat as handheld "Other" until the type grows one
        47 => Platform::Other("PlayStation Vita".into()),
        11 => Platform::Xbox(XboxGen::Original),
        12 => Platform::Xbox(XboxGen::Xbox360),
        49 => Platform::Xbox(XboxGen::XboxOne),
        169 => Platform::Xbox(XboxGen::SeriesXS),
        41 => Platform::Nintendo(NintendoPlatform::Wii),
        5 => Platform::Nintendo(NintendoPlatform::Snes),
        18 => Platform::Nintendo(NintendoPlatform::Nes),
        4 => Platform::Nintendo(NintendoPlatform::N64),
        21 => Platform::Nintendo(NintendoPlatform::GameBoy),
        22 => Platform::Nintendo(NintendoPlatform::GameBoyColor),
        24 => Platform::Nintendo(NintendoPlatform::GameBoyAdvance),
        20 => Platform::Nintendo(NintendoPlatform::Ds),
        37 => Platform::Nintendo(NintendoPlatform::ThreeDs),
        23 => Platform::Nintendo(NintendoPlatform::GameCube),
        130 => Platform::Nintendo(NintendoPlatform::Switch),
        415 => Platform::Nintendo(NintendoPlatform::Switch2),
        150 => Platform::Nintendo(NintendoPlatform::WiiU),
        _ => Platform::Other(format!("igdb:{id}")),
    }
}

fn platform_from_name(name: &str) -> Platform {
    let n = name.to_lowercase();
    if n.contains("pc") || n.contains("windows") {
        return Platform::Pc;
    }
    if n.contains("mac") {
        return Platform::Mac;
    }
    if n.contains("linux") {
        return Platform::Linux;
    }
    if n.contains("android") {
        return Platform::Android;
    }
    if n.contains("ios") || n.contains("iphone") {
        return Platform::Ios;
    }
    if n.contains("switch 2") {
        return Platform::Nintendo(NintendoPlatform::Switch2);
    }
    if n.contains("switch") {
        return Platform::Nintendo(NintendoPlatform::Switch);
    }
    if n.contains("playstation 5") || n.contains("ps5") {
        return Platform::PlayStation(5);
    }
    if n.contains("playstation 4") || n.contains("ps4") {
        return Platform::PlayStation(4);
    }
    if n.contains("playstation 3") || n.contains("ps3") {
        return Platform::PlayStation(3);
    }
    if n.contains("playstation 2") || n.contains("ps2") {
        return Platform::PlayStation(2);
    }
    if n.contains("playstation") {
        return Platform::PlayStation(1);
    }
    if n.contains("xbox series") {
        return Platform::Xbox(XboxGen::SeriesXS);
    }
    if n.contains("xbox one") {
        return Platform::Xbox(XboxGen::XboxOne);
    }
    if n.contains("xbox 360") {
        return Platform::Xbox(XboxGen::Xbox360);
    }
    if n.contains("xbox") {
        return Platform::Xbox(XboxGen::Original);
    }
    Platform::Other(name.to_string())
}

#[allow(clippy::match_same_arms)] // fallback for unknown IDs mirrors an existing variant intentionally
fn igdb_game_mode(id: u64) -> GameMode {
    match id {
        1 => GameMode::SinglePlayer,
        2 => GameMode::Multiplayer,
        3 => GameMode::CoOp,
        4 => GameMode::SplitScreen,
        5 => GameMode::Mmo,
        6 => GameMode::BattleRoyale,
        _ => GameMode::SinglePlayer,
    }
}

#[allow(clippy::match_same_arms)]
fn igdb_perspective(id: u64) -> PlayerPerspective {
    match id {
        1 => PlayerPerspective::FirstPerson,
        2 => PlayerPerspective::ThirdPerson,
        3 => PlayerPerspective::BirdViewIsometric,
        4 => PlayerPerspective::SideScroller,
        5 => PlayerPerspective::Text,
        6 => PlayerPerspective::Auditory,
        7 => PlayerPerspective::VirtualReality,
        _ => PlayerPerspective::ThirdPerson,
    }
}

fn igdb_status(raw: Option<u64>) -> ReleaseStatus {
    match raw {
        Some(0) => ReleaseStatus::Released,
        Some(2) => ReleaseStatus::Alpha,
        Some(3) => ReleaseStatus::Beta,
        Some(4) => ReleaseStatus::EarlyAccess,
        Some(5) => ReleaseStatus::Offline,
        Some(6) => ReleaseStatus::Cancelled,
        Some(7) => ReleaseStatus::Rumoured,
        Some(8) => ReleaseStatus::Delisted,
        _ => ReleaseStatus::Unknown,
    }
}

fn igdb_category(raw: Option<u64>) -> GameCategory {
    match raw {
        Some(0) => GameCategory::MainGame,
        Some(1) => GameCategory::DlcAddon,
        Some(2) => GameCategory::Expansion,
        Some(3) => GameCategory::Bundle,
        Some(4) => GameCategory::StandaloneExpansion,
        Some(5) => GameCategory::Mod,
        Some(6) => GameCategory::Episode,
        Some(7) => GameCategory::Season,
        Some(8) => GameCategory::Remake,
        Some(9) => GameCategory::Remaster,
        Some(10) => GameCategory::ExpandedGame,
        Some(11) => GameCategory::Port,
        Some(12) => GameCategory::Fork,
        _ => GameCategory::Unknown,
    }
}

fn igdb_age_rating(v: &serde_json::Value) -> AgeRating {
    let Some(arr) = v.as_array() else {
        return AgeRating::Unknown;
    };

    // Use the strictest rating found across all systems
    let mut strictest = AgeRating::Unknown;
    for r in arr {
        let cat = r["category"].as_u64().unwrap_or(0);
        let rating = r["rating"].as_u64().unwrap_or(0);

        let ar = match cat {
            // ESRB: 1=eC, 2=E, 3=E10+, 4=T, 5=M, 6=AO
            1 => match rating {
                1..=3 => AgeRating::AllAges,
                4 => AgeRating::Teen,
                5 => AgeRating::Mature,
                6 => AgeRating::AdultsOnly,
                _ => AgeRating::Unknown,
            },
            // PEGI: 1=3, 2=7, 3=12, 4=16, 5=18
            2 => match rating {
                1..=3 => AgeRating::AllAges,
                4 | 5 => AgeRating::Teen,
                6 => AgeRating::Mature,
                _ => AgeRating::Unknown,
            },
            // CERO: 1=A, 2=B, 3=C, 4=D, 5=Z
            3 => match rating {
                1 | 2 => AgeRating::AllAges,
                3 => AgeRating::Teen,
                4 => AgeRating::Mature,
                5 => AgeRating::AdultsOnly,
                _ => AgeRating::Unknown,
            },
            _ => AgeRating::Unknown,
        };

        if age_rank(ar) > age_rank(strictest) {
            strictest = ar;
        }
    }
    strictest
}

fn age_rank(a: AgeRating) -> u8 {
    match a {
        AgeRating::AllAges => 1,
        AgeRating::Teen => 2,
        AgeRating::Mature => 3,
        AgeRating::AdultsOnly => 4,
        AgeRating::Unknown => 0,
    }
}

fn igdb_region(id: u64) -> Region {
    match id {
        1 => Region::Europe,
        2 => Region::NorthAmerica,
        3 | 4 => Region::Australia,
        5 => Region::Japan,
        6 => Region::China,
        7 => Region::Asia,
        8 => Region::Worldwide,
        9 => Region::Korea,
        10 => Region::Brazil,
        _ => Region::Other,
    }
}

fn igdb_website_category(cat: u64) -> WebsiteKind {
    match cat {
        1 => WebsiteKind::Official,
        4 => WebsiteKind::Facebook,
        5 => WebsiteKind::Twitter,
        6 => WebsiteKind::Twitch,
        8 => WebsiteKind::Instagram,
        9 => WebsiteKind::Youtube,
        13 => WebsiteKind::Steam,
        14 => WebsiteKind::Reddit,
        15 => WebsiteKind::ItchIo,
        16 => WebsiteKind::EpicGames,
        17 => WebsiteKind::Gog,
        18 => WebsiteKind::Discord,
        _ => WebsiteKind::Other(format!("igdb:{cat}")),
    }
}
