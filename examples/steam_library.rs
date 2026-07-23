#![allow(clippy::doc_markdown)]
//! Scan the local Steam library and enrich each game with metadata.
//!
//! Requires the `steam` feature:
//!   `cargo run --example steam_library --features steam`
//!
//! Optional — add IGDB or TheGamesDB keys for richer fallback data:
//!   `IGDB_CLIENT_ID`, `IGDB_CLIENT_SECRET`, `TGDB_API_KEY`

#[cfg(feature = "steam")]
use gameinfo::{
    GameInfoClient, SteamLibrary,
    providers::{IgdbConfig, SteamConfig, TheGamesDbConfig},
};

#[cfg(feature = "steam")]
#[tokio::main]
async fn main() -> Result<(), gameinfo::Error> {
    tracing_subscriber::fmt::init();

    let mut builder = GameInfoClient::builder().provider(SteamConfig::default());

    if let (Ok(id), Ok(secret)) = (
        std::env::var("IGDB_CLIENT_ID"),
        std::env::var("IGDB_CLIENT_SECRET"),
    ) {
        builder = builder.provider(IgdbConfig {
            client_id: id,
            client_secret: secret,
        });
    }

    if let Ok(key) = std::env::var("TGDB_API_KEY") {
        builder = builder.provider(TheGamesDbConfig { api_key: key });
    }

    let client = builder.build();
    let library = SteamLibrary::new();

    println!("Scanning Steam libraries at:");
    for path in library.paths() {
        println!("  {}", path.display());
    }
    println!();

    let results = library.enrich_all(&client).await;

    println!("Found {} installed games:\n", results.len());
    for (installed, info) in &results {
        match info {
            Some(game) => println!(
                "  ✓ {} (app {}) — {} | {} | {}",
                game.title,
                installed.app_id,
                game.source,
                game.average_rating()
                    .map_or("no rating".into(), |r| format!("{r:.1}/100")),
                game.cover
                    .as_ref()
                    .map_or("No Cover".into(), |c| c.url.clone())
            ),
            None => println!(
                "  ? {} (app {}) — metadata unavailable",
                installed.name, installed.app_id
            ),
        }
    }

    Ok(())
}

#[cfg(not(feature = "steam"))]
fn main() {
    eprintln!("Run with --features steam");
}
