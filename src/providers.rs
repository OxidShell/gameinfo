pub mod igdb;
#[cfg(feature = "steam")]
pub mod steam;
pub mod thegamesdb;

pub use igdb::{IgdbConfig, IgdbProvider};
#[cfg(feature = "steam")]
pub use steam::{SteamConfig, SteamProvider};
pub use thegamesdb::{TheGamesDbConfig, TheGamesDbProvider};
