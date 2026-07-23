use std::collections::HashMap;

use async_trait::async_trait;
use tracing::instrument;

use crate::{
    error::Error,
    provider::{GameProvider, ProviderResult},
    query::SearchQuery,
    types::{
        AgeRating, Company, CompanyRole, ContentType, GameCategory, GameInfo, GameMode, Genre,
        Image, ImageKind, NintendoPlatform, Platform, PlayerCount, ProviderIds, ProviderKind,
        ReleaseStatus, Video, VideoKind, Website, WebsiteKind, XboxGen,
    },
};

const TGDB_API: &str = "https://api.thegamesdb.net/v1.1";

/// TheGamesDB API key.
///
/// Obtain at <https://forums.thegamesdb.net/viewforum.php?f=10>.
pub struct TheGamesDbConfig {
    pub api_key: String,
}

pub struct TheGamesDbProvider {
    client: reqwest::Client,
    config: TheGamesDbConfig,
    /// Lazily loaded genre ID → name map.
    genres_cache: tokio::sync::RwLock<Option<HashMap<u64, String>>>,
}

impl From<TheGamesDbConfig> for TheGamesDbProvider {
    fn from(config: TheGamesDbConfig) -> Self {
        Self::new(config)
    }
}

impl From<TheGamesDbConfig> for Box<dyn crate::provider::GameProvider> {
    fn from(config: TheGamesDbConfig) -> Self {
        Box::new(TheGamesDbProvider::from(config))
    }
}

impl TheGamesDbProvider {
    #[must_use]
    pub fn new(config: TheGamesDbConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
            genres_cache: tokio::sync::RwLock::new(None),
        }
    }

    async fn genre_name(&self, id: u64) -> String {
        // Try cache first
        {
            let guard = self.genres_cache.read().await;
            if let Some(ref map) = *guard
                && let Some(name) = map.get(&id)
            {
                return name.clone();
            }
        }

        // Fetch and populate cache
        if let Ok(json) = self.get("Genres", &[]).await
            && let Some(genres) = json["data"]["genres"].as_object()
        {
            let mut guard = self.genres_cache.write().await;
            let map = guard.get_or_insert_with(HashMap::new);
            for (_, g) in genres {
                if let (Some(gid), Some(gname)) = (g["id"].as_u64(), g["name"].as_str()) {
                    map.insert(gid, gname.to_string());
                }
            }
            if let Some(name) = map.get(&id) {
                return name.clone();
            }
        }

        format!("tgdb:{id}")
    }

    async fn get(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<serde_json::Value, Error> {
        let url = format!("{TGDB_API}/{endpoint}");
        let mut all_params: Vec<(&str, &str)> = Vec::with_capacity(params.len() + 1);
        all_params.push(("apikey", &self.config.api_key));
        all_params.extend_from_slice(params);

        let resp = self.client.get(&url).query(&all_params).send().await?;

        if resp.status().as_u16() == 429 {
            return Err(Error::RateLimit {
                provider: "TheGamesDB".into(),
            });
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let msg = resp.text().await.unwrap_or_default();
            return Err(Error::Provider {
                provider: "TheGamesDB".into(),
                message: format!("{status}: {msg}"),
            });
        }

        resp.json::<serde_json::Value>().await.map_err(Error::Http)
    }

    #[allow(clippy::too_many_lines)]
    async fn map_games(&self, json: &serde_json::Value, limit: usize) -> Vec<ProviderResult> {
        let Some(games) = json["data"]["games"].as_array() else {
            return Vec::new();
        };

        let base_url = json["include"]["boxart"]["base_url"]["original"]
            .as_str()
            .unwrap_or("https://cdn.thegamesdb.net/images/original/");

        let mut out = Vec::new();
        for game in games.iter().take(limit) {
            if let Some(result) = self.map_one(game, json, base_url).await {
                out.push(result);
            }
        }
        out
    }

    #[allow(clippy::too_many_lines)]
    async fn map_one(
        &self,
        game: &serde_json::Value,
        full: &serde_json::Value,
        img_base: &str,
    ) -> Option<ProviderResult> {
        let title = game["game_title"].as_str()?.to_string();
        let id = game["id"].as_u64()?;

        let ids = ProviderIds {
            thegamesdb: Some(id),
            ..Default::default()
        };

        let alternative_titles = game["alternates"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Genres come as IDs
        let genres = if let Some(arr) = game["genres"].as_array() {
            let mut g = Vec::new();
            for gid_val in arr {
                if let Some(gid) = gid_val.as_u64() {
                    let name = self.genre_name(gid).await;
                    g.push(tgdb_genre_name(&name));
                }
            }
            g
        } else {
            Vec::new()
        };

        // Platform
        let platform_id = game["platform"].as_u64().unwrap_or(0);
        let platform_name =
            full["include"]["platform"]["data"][platform_id.to_string().as_str()]["name"]
                .as_str()
                .unwrap_or("");
        let platform = tgdb_platform(platform_id, platform_name);

        // Images
        let id_str = id.to_string();
        let mut cover = None;
        let mut screenshots = Vec::new();
        let mut artworks = Vec::new();

        if let Some(boxarts) = full["include"]["boxart"]["data"][&id_str].as_array() {
            for art in boxarts {
                let Some(filename) = art["filename"].as_str() else {
                    continue;
                };
                let url = format!("{img_base}{filename}");
                let res = art["resolution"].as_str().unwrap_or("");
                let (w, h) = parse_resolution(res);

                match art["type"].as_str().unwrap_or("") {
                    "boxart" if art["side"].as_str() == Some("front") => {
                        cover = Some(Image {
                            url,
                            width: w,
                            height: h,
                            kind: ImageKind::Cover,
                        });
                    }
                    "fanart" => artworks.push(Image {
                        url,
                        width: w,
                        height: h,
                        kind: ImageKind::Artwork,
                    }),
                    "screenshot" => screenshots.push(Image {
                        url,
                        width: w,
                        height: h,
                        kind: ImageKind::Screenshot,
                    }),
                    "banner" => artworks.push(Image {
                        url,
                        width: w,
                        height: h,
                        kind: ImageKind::Banner,
                    }),
                    "clearlogo" => artworks.push(Image {
                        url,
                        width: w,
                        height: h,
                        kind: ImageKind::ClearLogo,
                    }),
                    _ => {}
                }
            }
        }

        // Release date
        let release_date = game["release_date"].as_str().and_then(|d| {
            time::Date::parse(
                d,
                &time::macros::format_description!("[year]-[month]-[day]"),
            )
            .ok()
        });

        // Players
        let player_count = game["players"]
            .as_str()
            .or_else(|| game["players"].as_u64().map(|_| ""))
            .and_then(|s| {
                s.parse::<u32>().ok().map(|max| PlayerCount {
                    min: 1,
                    max,
                    online_max: None,
                })
            });

        // Co-op
        let coop = game["coop"].as_str() == Some("Yes");
        let mut game_modes = vec![GameMode::SinglePlayer];
        if player_count.as_ref().is_some_and(|p| p.max > 1) {
            game_modes.push(GameMode::Multiplayer);
        }
        if coop {
            game_modes.push(GameMode::CoOp);
        }

        // Video (YouTube link)
        let videos = if let Some(yt) = game["youtube"].as_str().filter(|s| !s.is_empty()) {
            vec![Video {
                url: yt.to_string(),
                name: Some("Trailer".into()),
                kind: VideoKind::Trailer,
            }]
        } else {
            Vec::new()
        };

        // Credits (IDs, no easy resolution without extra calls)
        let developers: Vec<Company> = game["developers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        v.as_u64().map(|id| Company {
                            name: format!("tgdb-dev:{id}"),
                            role: CompanyRole::Developer,
                            url: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let publishers: Vec<Company> = game["publishers"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        v.as_u64().map(|id| Company {
                            name: format!("tgdb-pub:{id}"),
                            role: CompanyRole::Publisher,
                            url: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let age_rating = tgdb_rating(game["rating"].as_str().unwrap_or(""));

        // Official site / youtube as websites
        let mut websites: Vec<Website> = Vec::new();
        if let Some(yt) = game["youtube"].as_str().filter(|s| !s.is_empty()) {
            websites.push(Website {
                url: yt.to_string(),
                kind: WebsiteKind::Youtube,
            });
        }

        Some(ProviderResult {
            raw_score: 0.5, // TGDB doesn't expose a quality signal
            provider: ProviderKind::TheGamesDb,
            info: GameInfo {
                ids,
                source: ProviderKind::TheGamesDb,
                confidence: 0.0,
                title,
                alternative_titles,
                summary: game["overview"].as_str().map(String::from),
                storyline: None,
                slug: None,
                genres,
                themes: Vec::new(),
                keywords: Vec::new(),
                category: GameCategory::MainGame,
                status: ReleaseStatus::Released,
                content_type: ContentType::Game,
                age_rating,
                platforms: vec![platform],
                game_modes,
                player_perspectives: Vec::new(),
                player_count,
                languages: Vec::new(),
                file_formats: Vec::new(),
                release_date,
                platform_releases: Vec::new(),
                updated_at: game["last_updated"].as_str().and_then(|s| {
                    time::PrimitiveDateTime::parse(
                        s,
                        &time::macros::format_description!(
                            "[year]-[month]-[day] [hour]:[minute]:[second]"
                        ),
                    )
                    .ok()
                    .map(time::PrimitiveDateTime::assume_utc)
                }),
                developers,
                publishers,
                ratings: Vec::new(),
                cover,
                screenshots,
                artworks,
                videos,
                websites,
                price: None,
                download_count: None,
                file_size: None,
                franchise: None,
                series: Vec::new(),
                game_engines: Vec::new(),
                similar_games: Vec::new(),
                extra: HashMap::new(),
            },
        })
    }
}

#[async_trait]
impl GameProvider for TheGamesDbProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::TheGamesDb
    }

    #[instrument(skip(self), fields(provider = "TheGamesDB"))]
    async fn search(&self, query: &SearchQuery) -> Result<Vec<ProviderResult>, Error> {
        let limit = query.limit as usize;
        let json = self
            .get(
                "Games/ByGameName",
                &[
                    ("name", query.title.as_str()),
                    ("fields", "players,publishers,genres,overview,last_updated,rating,platform,coop,youtube,alternates"),
                    ("include", "boxart,platform"),
                ],
            )
            .await?;

        Ok(self.map_games(&json, limit).await)
    }

    #[instrument(skip(self), fields(provider = "TheGamesDB", id))]
    async fn fetch_by_id(&self, id: &str) -> Result<Option<GameInfo>, Error> {
        let json = self
            .get(
                "Games/ByGameID",
                &[
                    ("id", id),
                    ("fields", "players,publishers,genres,overview,last_updated,rating,platform,coop,youtube,alternates"),
                    ("include", "boxart,platform"),
                ],
            )
            .await?;

        Ok(self
            .map_games(&json, 1)
            .await
            .into_iter()
            .next()
            .map(|r| r.info))
    }
}

// ---------------------------------------------------------------------------
// Mapping helpers
// ---------------------------------------------------------------------------

fn tgdb_genre_name(name: &str) -> Genre {
    match name.to_lowercase().as_str() {
        s if s.contains("action") => Genre::Action,
        s if s.contains("adventure") => Genre::Adventure,
        s if s.contains("rpg") || s.contains("role") => Genre::RolePlaying,
        s if s.contains("strategy") => Genre::Strategy,
        s if s.contains("simulation") => Genre::Simulation,
        s if s.contains("puzzle") => Genre::Puzzle,
        s if s.contains("sport") => Genre::Sports,
        s if s.contains("racing") => Genre::Racing,
        s if s.contains("fighting") => Genre::Fighting,
        s if s.contains("shooter") => Genre::Shooter,
        s if s.contains("horror") => Genre::Horror,
        s if s.contains("visual novel") => Genre::VisualNovel,
        s if s.contains("platform") => Genre::Platformer,
        s if s.contains("stealth") => Genre::Stealth,
        s if s.contains("music") => Genre::MusicRhythm,
        s if s.contains("pinball") => Genre::Pinball,
        s if s.contains("quiz") || s.contains("trivia") => Genre::Quiz,
        s if s.contains("card") || s.contains("board") => Genre::CardGame,
        s if s.contains("mmo") => Genre::Moba,
        _ => Genre::Other(name.to_string()),
    }
}

fn tgdb_platform(id: u64, name: &str) -> Platform {
    // Try ID first, fall back to name
    match id {
        1 => Platform::Pc,
        37 => Platform::Android,
        38 => Platform::Ios,
        19 => Platform::PlayStation(1),
        20 => Platform::PlayStation(2),
        21 => Platform::PlayStation(3),
        34 => Platform::PlayStation(4),
        63 => Platform::PlayStation(5),
        22 => Platform::Xbox(XboxGen::Original),
        23 => Platform::Xbox(XboxGen::Xbox360),
        24 => Platform::Xbox(XboxGen::XboxOne),
        4828 => Platform::Xbox(XboxGen::SeriesXS),
        25 => Platform::Nintendo(NintendoPlatform::GameBoy),
        26 => Platform::Nintendo(NintendoPlatform::GameBoyColor),
        27 => Platform::Nintendo(NintendoPlatform::GameBoyAdvance),
        28 => Platform::Nintendo(NintendoPlatform::Ds),
        29 => Platform::Nintendo(NintendoPlatform::ThreeDs),
        30 => Platform::Nintendo(NintendoPlatform::Wii),
        31 => Platform::Nintendo(NintendoPlatform::WiiU),
        4924 => Platform::Nintendo(NintendoPlatform::Switch),
        _ => {
            let n = name.to_lowercase();
            if n.contains("pc") || n.contains("windows") {
                Platform::Pc
            } else if n.contains("mac") {
                Platform::Mac
            } else if n.contains("linux") {
                Platform::Linux
            } else if n.contains("android") {
                Platform::Android
            } else if n.contains("ios") {
                Platform::Ios
            } else {
                Platform::Other(name.to_string())
            }
        }
    }
}

fn tgdb_rating(r: &str) -> AgeRating {
    match r {
        "E" | "E10+" | "EC" | "3" | "7" => AgeRating::AllAges,
        "T" | "12" => AgeRating::Teen,
        "M" | "16" | "17" => AgeRating::Mature,
        "A" | "AO" | "18" => AgeRating::AdultsOnly,
        _ => AgeRating::Unknown,
    }
}

fn parse_resolution(res: &str) -> (Option<u32>, Option<u32>) {
    let parts: Vec<&str> = res.splitn(2, 'x').collect();
    if parts.len() == 2 {
        let w = parts[0].trim().parse().ok();
        let h = parts[1].trim().parse().ok();
        (w, h)
    } else {
        (None, None)
    }
}
