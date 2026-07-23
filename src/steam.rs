#![allow(clippy::module_name_repetitions)]

//! Local Steam library scanner.
//!
//! Reads `appmanifest_*.acf` files from Steam library directories and optionally
//! enriches each entry via a [`GameInfoClient`].

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::DateTime;
use tracing::warn;

use crate::{
    client::GameInfoClient,
    error::Error,
    query::SearchQuery,
    types::{GameInfo, ProviderKind},
};

/// A game found in a local Steam library.
#[derive(Debug, Clone)]
pub struct InstalledGame {
    pub app_id: u64,
    pub name: String,
    /// Name of the installation subdirectory inside `steamapps/common/`.
    pub install_dir: String,
    pub size_on_disk: Option<u64>,
    pub last_updated: Option<DateTime<chrono::Utc>>,
    pub build_id: Option<u64>,
}

/// Scans local Steam library directories and optionally enriches results via a provider client.
pub struct SteamLibrary {
    steamapps_paths: Vec<PathBuf>,
}

impl Default for SteamLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl SteamLibrary {
    /// Creates a scanner that auto-detects the default Steam library locations for the current OS.
    #[must_use]
    pub fn new() -> Self {
        Self {
            steamapps_paths: default_steamapps_paths(),
        }
    }

    /// Creates a scanner with explicit `steamapps` directory paths.
    #[must_use]
    pub fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            steamapps_paths: paths,
        }
    }

    /// Returns the configured steamapps directory paths.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.steamapps_paths
    }

    /// Scans all configured steamapps directories.
    ///
    /// Also reads `libraryfolders.vdf` in each directory to discover additional library locations.
    pub async fn scan(&self) -> Vec<InstalledGame> {
        let mut all_paths = self.steamapps_paths.clone();

        for base in &self.steamapps_paths {
            let vdf = base.join("libraryfolders.vdf");
            if let Ok(content) = tokio::fs::read_to_string(&vdf).await {
                for extra in parse_library_folders(&content) {
                    let candidate = extra.join("steamapps");
                    if !all_paths.contains(&candidate) {
                        all_paths.push(candidate);
                    }
                }
            }
        }

        let mut games = Vec::new();
        for dir in &all_paths {
            games.extend(scan_dir(dir).await);
        }
        games
    }

    /// Scans installed games and fetches metadata for each via `client`.
    ///
    /// Strategy per game:
    /// 1. `fetch_by_id(Steam, app_id)` — uses the Steam store API directly.
    /// 2. On rate limit, missing Steam provider, or any error: fall back to a title search
    ///    across all configured providers.
    ///
    /// A 500 ms pause is inserted between requests to respect Steam's undocumented rate limits.
    pub async fn enrich_all(
        &self,
        client: &GameInfoClient,
    ) -> Vec<(InstalledGame, Option<GameInfo>)> {
        let games = self.scan().await;
        let mut results = Vec::with_capacity(games.len());

        for (i, game) in games.iter().enumerate() {
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            let info = Self::enrich_one(client, game).await;
            results.push((game.clone(), info));
        }

        results
    }

    async fn enrich_one(client: &GameInfoClient, game: &InstalledGame) -> Option<GameInfo> {
        let id_str = game.app_id.to_string();

        match client.fetch_by_id(ProviderKind::Steam, &id_str).await {
            Ok(Some(info)) => return Some(info),
            Ok(None) | Err(Error::ProviderNotFound { .. }) => {}
            Err(e) => {
                warn!(
                    app_id = game.app_id,
                    error = %e,
                    "Steam fetch failed, falling back to title search"
                );
            }
        }

        let query = SearchQuery::new(&game.name)
            .with_steam_app_id(game.app_id)
            .with_limit(1);

        client
            .search(&query)
            .await
            .ok()
            .and_then(|r| r.into_iter().next())
    }
}

// ---------------------------------------------------------------------------
// Filesystem scanning
// ---------------------------------------------------------------------------

async fn scan_dir(dir: &Path) -> Vec<InstalledGame> {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return Vec::new();
    };

    let mut games = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let is_manifest = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("acf"))
            && name.starts_with("appmanifest_");
        if !is_manifest {
            continue;
        }
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        if let Some(game) = parse_appmanifest(&content) {
            games.push(game);
        }
    }

    games
}

fn parse_appmanifest(content: &str) -> Option<InstalledGame> {
    let map = parse_vdf_flat(content);

    let app_id = map.get("appid")?.parse::<u64>().ok()?;
    let name = map.get("name")?.clone();
    let install_dir = map.get("installdir").cloned().unwrap_or_default();
    let size_on_disk = map.get("SizeOnDisk").and_then(|s| s.parse().ok());
    let build_id = map.get("buildid").and_then(|s| s.parse().ok());
    let last_updated = map
        .get("LastUpdated")
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|ts| DateTime::from_timestamp(ts, 0));

    Some(InstalledGame {
        app_id,
        name,
        install_dir,
        size_on_disk,
        last_updated,
        build_id,
    })
}

/// Extracts additional Steam library root paths from `libraryfolders.vdf`.
fn parse_library_folders(content: &str) -> Vec<PathBuf> {
    content
        .lines()
        .filter_map(|line| {
            let (k, v) = parse_quoted_pair(line.trim())?;
            if k == "path" {
                Some(PathBuf::from(v))
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// VDF / ACF parser (flat key-value extraction only)
// ---------------------------------------------------------------------------

fn parse_vdf_flat(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        if let Some((k, v)) = parse_quoted_pair(line.trim()) {
            map.entry(k).or_insert(v);
        }
    }
    map
}

fn parse_quoted_pair(line: &str) -> Option<(String, String)> {
    let (key, rest) = consume_quoted(line)?;
    let (value, _) = consume_quoted(rest.trim_start())?;
    Some((key, value))
}

/// Reads one double-quoted VDF token, returning `(content, remaining_input)`.
/// Handles `\"` escape sequences.
fn consume_quoted(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = s.char_indices();
    loop {
        let (i, c) = chars.next()?;
        match c {
            '\\' => {
                if let Some((_, escaped)) = chars.next() {
                    out.push(escaped);
                }
            }
            '"' => return Some((out, &s[i + 1..])),
            _ => out.push(c),
        }
    }
}

// ---------------------------------------------------------------------------
// Default OS paths
// ---------------------------------------------------------------------------

fn default_steamapps_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "linux")]
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".steam/steam/steamapps"));
        paths.push(home.join(".local/share/Steam/steamapps"));
    }

    #[cfg(target_os = "windows")]
    {
        paths.push(PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps"));
        paths.push(PathBuf::from(r"C:\Program Files\Steam\steamapps"));
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join("Library/Application Support/Steam/steamapps"));
    }

    paths
}
