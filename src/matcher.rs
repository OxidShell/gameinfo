use std::collections::HashSet;

use crate::{
    normalize::{normalize, tokenize},
    provider::ProviderResult,
    types::{
        Company, GameInfo, Image, Language, PlatformRelease, ProviderIds, ProviderKind, Rating,
        Video, Website,
    },
};

/// Tuning parameters for the smart-matching algorithm.
#[derive(Debug, Clone)]
pub struct MatcherConfig {
    /// Results below this confidence are dropped (unless `include_low_confidence` is set).
    pub min_confidence: f64,
    /// Weight given to title similarity vs. the provider's own relevance signal.
    pub title_weight: f64,
    /// When true, low-confidence results are included but sorted last.
    pub include_low_confidence: bool,
    /// Jaro-Winkler threshold above which two titles are considered duplicates.
    pub dedup_threshold: f64,
}

impl Default for MatcherConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.40,
            title_weight: 0.75,
            include_low_confidence: false,
            dedup_threshold: 0.90,
        }
    }
}

pub struct SmartMatcher {
    config: MatcherConfig,
}

impl SmartMatcher {
    #[must_use]
    pub fn new(config: MatcherConfig) -> Self {
        Self { config }
    }

    /// Score one provider result against the user's query title.
    #[must_use]
    pub fn score(&self, query: &str, result: &ProviderResult) -> f64 {
        let q = normalize(query);
        let t = normalize(&result.info.title);

        let jw = strsim::jaro_winkler(&q, &t);

        // Jaccard over significant tokens
        let q_tok: HashSet<String> = tokenize(query).into_iter().collect();
        let t_tok: HashSet<String> = tokenize(&result.info.title).into_iter().collect();
        let jaccard = if q_tok.is_empty() && t_tok.is_empty() {
            1.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let inter = q_tok.intersection(&t_tok).count() as f64;
            #[allow(clippy::cast_precision_loss)]
            let union = q_tok.union(&t_tok).count() as f64;
            inter / union
        };

        let exact = if q == t { 0.15_f64 } else { 0.0 };

        // Boost for a match against an alternative title (weighted half)
        let alt_boost = result
            .info
            .alternative_titles
            .iter()
            .map(|alt| strsim::jaro_winkler(&q, &normalize(alt)))
            .fold(0.0_f64, f64::max)
            * 0.5;

        let title_score = (jw * 0.6 + jaccard * 0.4 + exact + alt_boost).min(1.0);

        (title_score * self.config.title_weight
            + result.raw_score * (1.0 - self.config.title_weight))
            .min(1.0)
    }

    /// Score, filter, and sort a flat list of provider results.
    #[must_use]
    pub fn rank(&self, query: &str, results: Vec<ProviderResult>) -> Vec<GameInfo> {
        let mut scored: Vec<(f64, GameInfo)> = results
            .into_iter()
            .map(|r| {
                let s = self.score(query, &r);
                let mut info = r.info;
                info.confidence = s;
                (s, info)
            })
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        if self.config.include_low_confidence {
            scored.into_iter().map(|(_, g)| g).collect()
        } else {
            scored
                .into_iter()
                .filter(|(s, _)| *s >= self.config.min_confidence)
                .map(|(_, g)| g)
                .collect()
        }
    }

    /// Deduplicate a ranked list by merging near-duplicate entries.
    ///
    /// When two titles exceed `dedup_threshold` in Jaro-Winkler similarity,
    /// the higher-confidence entry is kept and enriched with fields from the other.
    #[must_use]
    pub fn dedup_merge(&self, ranked: Vec<GameInfo>) -> Vec<GameInfo> {
        let mut out: Vec<GameInfo> = Vec::with_capacity(ranked.len());

        'next: for candidate in ranked {
            let cn = normalize(&candidate.title);
            for existing in &mut out {
                if strsim::jaro_winkler(&cn, &normalize(&existing.title))
                    >= self.config.dedup_threshold
                {
                    // Clone is unavoidable here: we need to take ownership of
                    // both `existing` and `candidate` to call merge(), but
                    // `existing` lives behind `&mut` in the outer Vec.
                    #[allow(clippy::assigning_clones)]
                    if candidate.confidence > existing.confidence {
                        *existing = merge(candidate, existing.clone());
                    } else {
                        *existing = merge(existing.clone(), candidate);
                    }
                    continue 'next;
                }
            }
            out.push(candidate);
        }

        out
    }
}

/// Merge `primary` (higher confidence) with `secondary`, filling gaps.
pub(crate) fn merge(mut primary: GameInfo, secondary: GameInfo) -> GameInfo {
    merge_ids(&mut primary.ids, &secondary.ids);

    if primary.summary.is_none() {
        primary.summary = secondary.summary;
    }
    if primary.storyline.is_none() {
        primary.storyline = secondary.storyline;
    }
    if primary.cover.is_none() {
        primary.cover = secondary.cover;
    }
    if primary.release_date.is_none() {
        primary.release_date = secondary.release_date;
    }
    if primary.price.is_none() {
        primary.price = secondary.price;
    }
    if primary.download_count.is_none() {
        primary.download_count = secondary.download_count;
    }
    if primary.file_size.is_none() {
        primary.file_size = secondary.file_size;
    }
    if primary.franchise.is_none() {
        primary.franchise = secondary.franchise;
    }
    if primary.player_count.is_none() {
        primary.player_count = secondary.player_count;
    }
    if primary.updated_at.is_none() {
        primary.updated_at = secondary.updated_at;
    }
    if primary.slug.is_none() {
        primary.slug = secondary.slug;
    }

    merge_vec(&mut primary.genres, secondary.genres);
    merge_vec(&mut primary.platforms, secondary.platforms);
    merge_vec(&mut primary.game_modes, secondary.game_modes);
    merge_vec(
        &mut primary.player_perspectives,
        secondary.player_perspectives,
    );
    merge_vec(&mut primary.themes, secondary.themes);
    merge_vec(&mut primary.keywords, secondary.keywords);
    merge_vec::<Image>(&mut primary.screenshots, secondary.screenshots);
    merge_vec::<Image>(&mut primary.artworks, secondary.artworks);
    merge_vec::<Video>(&mut primary.videos, secondary.videos);
    merge_vec::<Website>(&mut primary.websites, secondary.websites);
    merge_vec::<Company>(&mut primary.developers, secondary.developers);
    merge_vec::<Company>(&mut primary.publishers, secondary.publishers);
    merge_vec::<Rating>(&mut primary.ratings, secondary.ratings);
    merge_vec(&mut primary.game_engines, secondary.game_engines);
    merge_vec(&mut primary.series, secondary.series);
    merge_vec::<Language>(&mut primary.languages, secondary.languages);
    merge_vec::<PlatformRelease>(&mut primary.platform_releases, secondary.platform_releases);
    merge_vec(&mut primary.file_formats, secondary.file_formats);
    merge_vec(&mut primary.similar_games, secondary.similar_games);

    for alt in secondary.alternative_titles {
        if alt != primary.title && !primary.alternative_titles.contains(&alt) {
            primary.alternative_titles.push(alt);
        }
    }

    for (k, v) in secondary.extra {
        primary.extra.entry(k).or_insert(v);
    }

    primary.source = ProviderKind::Merged;
    primary
}

// clone_from would unconditionally overwrite; guarding with is_none() is intentional
#[allow(clippy::assigning_clones)]
fn merge_ids(target: &mut ProviderIds, src: &ProviderIds) {
    if target.igdb.is_none() {
        target.igdb = src.igdb;
    }
    if target.thegamesdb.is_none() {
        target.thegamesdb = src.thegamesdb;
    }
    if target.steam.is_none() {
        target.steam = src.steam;
    }
    if target.gog.is_none() {
        target.gog = src.gog.clone();
    }
    for (k, v) in &src.external {
        target
            .external
            .entry(k.clone())
            .or_insert_with(|| v.clone());
    }
}

fn merge_vec<T: PartialEq>(target: &mut Vec<T>, source: Vec<T>) {
    for item in source {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}
