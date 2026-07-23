#![allow(clippy::module_name_repetitions)]

//! Steam provider — uses `store.steampowered.com/api`.
//!
//! `appdetails` is semi-official (stable, used by the Steam client itself).
//! `storesearch` is undocumented and may change without notice.

use std::collections::HashMap;

use async_trait::async_trait;
use tracing::instrument;

use crate::{
    error::Error,
    provider::{GameProvider, ProviderResult},
    query::SearchQuery,
    types::{
        AgeRating, Company, CompanyRole, ContentType, GameCategory, GameInfo, GameMode, Genre,
        Image, ImageKind, Language, LanguageSupport, Platform, Price, ProviderIds, ProviderKind,
        Rating, RatingKind, ReleaseStatus, Video, VideoKind, Website, WebsiteKind,
    },
};

const STORE_API: &str = "https://store.steampowered.com/api";

/// Configuration for the Steam metadata provider.
pub struct SteamConfig {
    /// ISO 3166-1 alpha-2 country code used for price localisation. Defaults to `"us"`.
    pub country_code: String,
}

impl Default for SteamConfig {
    fn default() -> Self {
        Self {
            country_code: "us".to_string(),
        }
    }
}

pub struct SteamProvider {
    client: reqwest::Client,
    config: SteamConfig,
}

impl From<SteamConfig> for SteamProvider {
    fn from(config: SteamConfig) -> Self {
        Self::new(config)
    }
}

impl From<SteamConfig> for Box<dyn crate::provider::GameProvider> {
    fn from(config: SteamConfig) -> Self {
        Box::new(SteamProvider::from(config))
    }
}

impl SteamProvider {
    #[must_use]
    pub fn new(config: SteamConfig) -> Self {
        Self {
            client: reqwest::Client::new(),
            config,
        }
    }

    async fn get(&self, path: &str, params: &[(&str, &str)]) -> Result<serde_json::Value, Error> {
        let url = format!("{STORE_API}/{path}");
        let resp = self.client.get(&url).query(params).send().await?;
        if resp.status().as_u16() == 429 {
            return Err(Error::RateLimit {
                provider: "Steam".into(),
            });
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let msg = resp.text().await.unwrap_or_default();
            return Err(Error::Provider {
                provider: "Steam".into(),
                message: format!("{status}: {msg}"),
            });
        }
        resp.json::<serde_json::Value>().await.map_err(Error::Http)
    }

    async fn fetch_details(&self, ids: &[u64]) -> Result<serde_json::Value, Error> {
        let ids_str = ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.get(
            "appdetails",
            &[
                ("appids", ids_str.as_str()),
                ("cc", self.config.country_code.as_str()),
                ("l", "en"),
            ],
        )
        .await
    }
}

#[async_trait]
impl GameProvider for SteamProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Steam
    }

    #[instrument(skip(self), fields(provider = "Steam"))]
    async fn search(&self, query: &SearchQuery) -> Result<Vec<ProviderResult>, Error> {
        let limit = query.limit as usize;

        let search_json = self
            .get(
                "storesearch/",
                &[
                    ("term", query.title.as_str()),
                    ("cc", self.config.country_code.as_str()),
                    ("l", "en"),
                ],
            )
            .await?;

        let Some(items) = search_json["items"].as_array() else {
            return Ok(Vec::new());
        };

        let ids: Vec<u64> = items
            .iter()
            .take(limit)
            .filter_map(|v| v["id"].as_u64())
            .collect();

        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let details = self.fetch_details(&ids).await?;

        let results = ids
            .iter()
            .filter_map(|id| {
                let entry = &details[id.to_string().as_str()];
                if entry["success"].as_bool() != Some(true) {
                    return None;
                }
                map_appdetails(&entry["data"], *id)
            })
            .collect();

        Ok(results)
    }

    #[instrument(skip(self), fields(provider = "Steam", id))]
    async fn fetch_by_id(&self, id: &str) -> Result<Option<GameInfo>, Error> {
        let app_id: u64 = id.parse().map_err(|_| Error::Provider {
            provider: "Steam".into(),
            message: format!("invalid app ID: {id}"),
        })?;

        let details = self.fetch_details(&[app_id]).await?;
        let entry = &details[id];
        if entry["success"].as_bool() != Some(true) {
            return Ok(None);
        }

        Ok(map_appdetails(&entry["data"], app_id).map(|r| r.info))
    }
}

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn map_appdetails(data: &serde_json::Value, app_id: u64) -> Option<ProviderResult> {
    let title = data["name"].as_str()?.to_string();

    let ids = ProviderIds {
        steam: Some(app_id),
        ..Default::default()
    };

    let genres: Vec<Genre> = data["genres"].as_array().map_or(Vec::new(), |arr| {
        arr.iter()
            .filter_map(|g| g["description"].as_str().map(steam_genre))
            .collect()
    });

    let mut platforms = Vec::new();
    if data["platforms"]["windows"].as_bool() == Some(true) {
        platforms.push(Platform::Pc);
    }
    if data["platforms"]["mac"].as_bool() == Some(true) {
        platforms.push(Platform::Mac);
    }
    if data["platforms"]["linux"].as_bool() == Some(true) {
        platforms.push(Platform::Linux);
    }

    let mut game_modes: Vec<GameMode> = Vec::new();
    if let Some(cats) = data["categories"].as_array() {
        for cat in cats {
            match cat["id"].as_u64().unwrap_or(0) {
                2 if !game_modes.contains(&GameMode::SinglePlayer) => {
                    game_modes.push(GameMode::SinglePlayer);
                }
                1 | 36 | 37 if !game_modes.contains(&GameMode::Multiplayer) => {
                    game_modes.push(GameMode::Multiplayer);
                }
                9 | 38 if !game_modes.contains(&GameMode::CoOp) => {
                    game_modes.push(GameMode::CoOp);
                }
                24 if !game_modes.contains(&GameMode::SplitScreen) => {
                    game_modes.push(GameMode::SplitScreen);
                }
                _ => {}
            }
        }
    }
    if game_modes.is_empty() {
        game_modes.push(GameMode::SinglePlayer);
    }

    let (category, content_type) = match data["type"].as_str().unwrap_or("game") {
        "dlc" => (GameCategory::DlcAddon, ContentType::Game),
        "demo" => (GameCategory::Unknown, ContentType::Other),
        "mod" => (GameCategory::Mod, ContentType::Game),
        "episode" => (GameCategory::Episode, ContentType::Game),
        _ => (GameCategory::MainGame, ContentType::Game),
    };

    let cover = data["header_image"].as_str().map(|url| Image {
        url: url.to_string(),
        width: Some(460),
        height: Some(215),
        kind: ImageKind::Cover,
    });

    let screenshots: Vec<Image> = data["screenshots"].as_array().map_or(Vec::new(), |arr| {
        arr.iter()
            .filter_map(|s| {
                s["path_full"].as_str().map(|url| Image {
                    url: url.to_string(),
                    width: None,
                    height: None,
                    kind: ImageKind::Screenshot,
                })
            })
            .collect()
    });

    let videos: Vec<Video> = data["movies"].as_array().map_or(Vec::new(), |arr| {
        arr.iter()
            .filter_map(|m| {
                let url = m["mp4"]["max"].as_str().or(m["webm"]["max"].as_str())?;
                Some(Video {
                    url: url.to_string(),
                    name: m["name"].as_str().map(String::from),
                    kind: VideoKind::Trailer,
                })
            })
            .collect()
    });

    let developers: Vec<Company> = data["developers"].as_array().map_or(Vec::new(), |arr| {
        arr.iter()
            .filter_map(|v| {
                v.as_str().map(|name| Company {
                    name: name.to_string(),
                    role: CompanyRole::Developer,
                    url: None,
                })
            })
            .collect()
    });

    let publishers: Vec<Company> = data["publishers"].as_array().map_or(Vec::new(), |arr| {
        arr.iter()
            .filter_map(|v| {
                v.as_str().map(|name| Company {
                    name: name.to_string(),
                    role: CompanyRole::Publisher,
                    url: None,
                })
            })
            .collect()
    });

    #[allow(clippy::cast_precision_loss)]
    let ratings: Vec<Rating> = data["metacritic"]["score"]
        .as_u64()
        .map(|score| {
            vec![Rating {
                score: score as f64,
                count: None,
                source: ProviderKind::Steam,
                kind: RatingKind::Critic,
            }]
        })
        .unwrap_or_default();

    let release_date = data["release_date"]["date"]
        .as_str()
        .and_then(parse_steam_date);

    let status = if data["release_date"]["coming_soon"].as_bool() == Some(true) {
        ReleaseStatus::Unknown
    } else {
        ReleaseStatus::Released
    };

    let age_rating = {
        let required_age = data["required_age"].as_u64().unwrap_or(0);
        let has_sexual = data["content_descriptors"]["ids"]
            .as_array()
            .is_some_and(|ids| ids.iter().any(|id| id.as_u64() == Some(5)));
        if has_sexual || required_age >= 18 {
            AgeRating::AdultsOnly
        } else if required_age >= 16 {
            AgeRating::Mature
        } else if required_age >= 12 {
            AgeRating::Teen
        } else {
            AgeRating::AllAges
        }
    };

    let price = map_price(data);
    let languages = data["supported_languages"]
        .as_str()
        .map_or(Vec::new(), parse_steam_languages);

    let mut websites = vec![Website {
        url: format!("https://store.steampowered.com/app/{app_id}/"),
        kind: WebsiteKind::Steam,
    }];
    if let Some(site) = data["website"].as_str().filter(|s| !s.is_empty()) {
        websites.push(Website {
            url: site.to_string(),
            kind: WebsiteKind::Official,
        });
    }

    let summary = data["short_description"].as_str().map(String::from);
    let storyline = data["about_the_game"]
        .as_str()
        .map(|s| strip_html(s).trim().to_string())
        .filter(|s| !s.is_empty());

    Some(ProviderResult {
        raw_score: 0.6,
        provider: ProviderKind::Steam,
        info: GameInfo {
            ids,
            source: ProviderKind::Steam,
            confidence: 0.0,
            title,
            alternative_titles: Vec::new(),
            summary,
            storyline,
            slug: None,
            genres,
            themes: Vec::new(),
            keywords: Vec::new(),
            category,
            status,
            content_type,
            age_rating,
            platforms,
            game_modes,
            player_perspectives: Vec::new(),
            player_count: None,
            languages,
            file_formats: Vec::new(),
            release_date,
            platform_releases: Vec::new(),
            updated_at: None,
            developers,
            publishers,
            ratings,
            cover,
            screenshots,
            artworks: Vec::new(),
            videos,
            websites,
            price,
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

fn map_price(data: &serde_json::Value) -> Option<Price> {
    if data["is_free"].as_bool() == Some(true) {
        return Some(Price {
            amount: 0.0,
            currency: "USD".to_string(),
            sale_amount: None,
            on_sale: false,
        });
    }
    let price = data["price_overview"].as_object()?;
    let currency = price
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("USD")
        .to_string();
    #[allow(clippy::cast_precision_loss)]
    let initial = price
        .get("initial")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as f64
        / 100.0;
    #[allow(clippy::cast_precision_loss)]
    let discounted = price
        .get("final")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as f64
        / 100.0;
    let on_sale = initial > discounted && discounted > 0.0;
    Some(Price {
        amount: initial,
        currency,
        sale_amount: if on_sale { Some(discounted) } else { None },
        on_sale,
    })
}

fn steam_genre(desc: &str) -> Genre {
    match desc {
        "Action" => Genre::Action,
        "Adventure" => Genre::Adventure,
        "RPG" => Genre::RolePlaying,
        "Strategy" => Genre::Strategy,
        "Simulation" => Genre::Simulation,
        "Sports" => Genre::Sports,
        "Racing" => Genre::Racing,
        "Puzzle" => Genre::Puzzle,
        "Indie" => Genre::Indie,
        "Platformer" => Genre::Platformer,
        "Fighting" => Genre::Fighting,
        "Shooter" => Genre::Shooter,
        "Casual" => Genre::Arcade,
        "Card Game" => Genre::CardGame,
        _ => Genre::Other(desc.to_string()),
    }
}

fn parse_steam_date(s: &str) -> Option<chrono::NaiveDate> {
    use chrono::NaiveDate;
    // "9 Jul, 2013"
    if let Ok(d) = NaiveDate::parse_from_str(s, "%e %b, %Y") {
        return Some(d);
    }
    // "Sep 20, 2021" (US format)
    if let Ok(d) = NaiveDate::parse_from_str(s, "%b %e, %Y") {
        return Some(d);
    }
    // "Jul 2013"
    if let Ok(d) = NaiveDate::parse_from_str(&format!("1 {s}"), "%e %b %Y") {
        return Some(d);
    }
    // "2013"
    s.trim()
        .parse::<i32>()
        .ok()
        .and_then(|y| NaiveDate::from_ymd_opt(y, 1, 1))
}

fn parse_steam_languages(html: &str) -> Vec<Language> {
    html.split(',')
        .filter_map(|part| {
            let raw = strip_html(part.trim());
            let has_asterisk = raw.contains('*');
            let name = raw.replace('*', "");
            let name = name.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let kind = if has_asterisk {
                LanguageSupport::Interface
            } else {
                LanguageSupport::Full
            };
            let (code, canonical) = lang_name_to_code(&name);
            Some(Language {
                code,
                name: canonical,
                kind,
            })
        })
        .collect()
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn lang_name_to_code(name: &str) -> (String, String) {
    let code = match name {
        "English" => "en",
        "French" => "fr",
        "German" => "de",
        "Spanish - Spain" | "Spanish" | "Spanish - Latin America" => "es",
        "Portuguese - Brazil" | "Portuguese" | "Portuguese - Portugal" => "pt",
        "Italian" => "it",
        "Dutch" => "nl",
        "Russian" => "ru",
        "Japanese" => "ja",
        "Korean" => "ko",
        "Simplified Chinese" | "Traditional Chinese" => "zh",
        "Polish" => "pl",
        "Turkish" => "tr",
        "Czech" => "cs",
        "Hungarian" => "hu",
        "Romanian" => "ro",
        "Swedish" => "sv",
        "Norwegian" => "no",
        "Danish" => "da",
        "Finnish" => "fi",
        "Ukrainian" => "uk",
        "Arabic" => "ar",
        "Thai" => "th",
        "Vietnamese" => "vi",
        "Greek" => "el",
        "Bulgarian" => "bg",
        _ => "und",
    };
    (code.to_string(), name.to_string())
}
