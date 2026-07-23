# gameinfo

Unified game metadata library for Rust. Queries multiple providers in parallel, then ranks,
deduplicates and merges results into a single `GameInfo` per title.

## Providers

| Provider     | Type          | Auth required      | Notes                              |
|--------------|---------------|--------------------|------------------------------------|
| IGDB         | REST/Apicalypse | Twitch OAuth2    | Most complete metadata             |
| TheGamesDB   | REST          | API key            | Good retro game coverage           |
| Steam        | REST (semi-official) | None      | `--features steam` required        |

## Quick start

```toml
[dependencies]
gameinfo = { path = "…" }

# Optional: local Steam library scanning
gameinfo = { path = "…", features = ["steam"] }
```

```rust
use gameinfo::{GameInfoClient, SearchQuery, providers::{IgdbConfig, TheGamesDbConfig}};

let client = GameInfoClient::builder()
    .provider(IgdbConfig {
        client_id:     std::env::var("IGDB_CLIENT_ID").unwrap(),
        client_secret: std::env::var("IGDB_CLIENT_SECRET").unwrap(),
    })
    .provider(TheGamesDbConfig {
        api_key: std::env::var("TGDB_API_KEY").unwrap(),
    })
    .build();

let results = client.search(&SearchQuery::new("Hollow Knight")).await?;

for game in &results {
    println!("[{:.0}%] {} — {:?}", game.confidence * 100.0, game.title, game.genres);
}
```

## Steam feature

Scan locally installed Steam games and enrich them with metadata:

```rust
use gameinfo::{GameInfoClient, SteamLibrary, providers::SteamConfig};

let client = GameInfoClient::builder()
    .provider(SteamConfig::default())
    .build();

// Vec<(InstalledGame, Option<GameInfo>)>
let results = SteamLibrary::new().enrich_all(&client).await;
```

`enrich_all` tries the Steam store API first; falls back to a title search across all configured
providers on rate limit or failure. A 500 ms pause is inserted between requests.

## Smart matching

Results are ranked using Jaro-Winkler similarity + Jaccard token overlap, then near-duplicates
(≥ 90 % title similarity) are merged — the highest-confidence entry wins, the lower one fills
any missing fields.

Tune via `MatcherConfig`:

```rust
use gameinfo::MatcherConfig;

let client = GameInfoClient::builder()
    // …providers…
    .matcher_config(MatcherConfig {
        min_confidence: 0.5,
        dedup_threshold: 0.85,
        ..Default::default()
    })
    .build();
```

## Running examples

```sh
# Multi-provider title search
cargo run --example search -- "Hollow Knight"

# Local Steam library scan (requires steam feature)
cargo run --example steam_library --features steam
```

Set `IGDB_CLIENT_ID` / `IGDB_CLIENT_SECRET` (from [Twitch Dev Console](https://dev.twitch.tv/console))
and `TGDB_API_KEY` (from [TheGamesDB forum](https://forums.thegamesdb.net/viewforum.php?f=10)).
