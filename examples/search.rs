//! Basic multi-provider search.
//!
//! Set environment variables before running:
//!   `IGDB_CLIENT_ID`, `IGDB_CLIENT_SECRET`  (<https://dev.twitch.tv/console>)
//!   `TGDB_API_KEY`                          (<https://forums.thegamesdb.net>)
//!
//! Run:
//!   cargo run --example search -- "Hollow Knight"

use gameinfo::{
    GameInfoClient, SearchQuery,
    providers::{IgdbConfig, TheGamesDbConfig},
};

#[tokio::main]
async fn main() -> Result<(), gameinfo::Error> {
    tracing_subscriber::fmt::init();

    let title = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Hollow Knight".into());

    let mut builder = GameInfoClient::builder();

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
    let query = SearchQuery::new(&title).with_limit(5);
    let results = client.search(&query).await?;

    if results.is_empty() {
        println!("No results for '{title}'");
        return Ok(());
    }

    for game in &results {
        println!(
            "[{:.0}%] {} ({}) — {} | {}",
            game.confidence * 100.0,
            game.title,
            game.release_date
                .map_or("?".into(), |d| d.format("%Y").to_string()),
            game.source,
            game.average_rating()
                .map_or("no rating".into(), |r| format!("{r:.1}/100")),
        );

        if let Some(summary) = &game.summary {
            let truncated: String = summary.chars().take(120).collect();
            println!("    {truncated}…");
        }

        if !game.genres.is_empty() {
            let genres: Vec<String> = game.genres.iter().map(|g| format!("{g:?}")).collect();
            println!("    Genres: {}", genres.join(", "));
        }

        println!();
    }

    Ok(())
}
