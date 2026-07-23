use crate::types::Platform;

/// Parameters for a game search across providers.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub title: String,
    pub platform: Option<Platform>,
    /// Release year filter.
    pub year: Option<u32>,
    /// Max results per provider.
    pub limit: u32,
    /// Steam App ID hint — lets providers cross-reference by Steam ID.
    pub steam_app_id: Option<u64>,
}

impl SearchQuery {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            platform: None,
            year: None,
            limit: 10,
            steam_app_id: None,
        }
    }

    #[must_use]
    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.platform = Some(platform);
        self
    }

    #[must_use]
    pub fn with_year(mut self, year: u32) -> Self {
        self.year = Some(year);
        self
    }

    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    /// Attach a Steam App ID hint for providers that support cross-referencing by Steam ID.
    #[must_use]
    pub fn with_steam_app_id(mut self, id: u64) -> Self {
        self.steam_app_id = Some(id);
        self
    }
}
