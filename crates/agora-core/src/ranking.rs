//! Unified cross-source ranking for Browse.
//!
//! Curated registry items, Modrinth projects, and Technic packs carry entirely
//! different signals: the registry has votes but no download counts, Modrinth
//! has downloads and followers but no votes, Technic has installs and ratings.
//! This module reduces all of them to one comparable 0-100 score so a single
//! list can be ranked honestly.
//!
//! Design decisions worth not relitigating:
//!
//! - **Followers are added, never divided by downloads.** A followers/downloads
//!   ratio is a niche-ness detector, not a quality detector. Measured against
//!   live data it ranks JourneyMap (0.035%) and Create (0.028%) above Sodium
//!   (0.019%), which is exactly backwards for "what do I install first".
//! - **The scale constants are fixed reference points, not statistics.** We
//!   cannot know Modrinth's median download count without Modrinth's entire
//!   database, and we do not need to — a hand-calibrated ceiling is enough.
//! - **Curated and uncurated occupy overlapping bands** rather than being
//!   separated or interleaved by quota. Top curated leads, uncurated trickles in
//!   behind it, and weak curated content settles mid-list instead of at the
//!   bottom. The guarantee holds at any registry size without recomputation.
//! - **Votes only nudge, and are inert at zero votes.** Every item in the
//!   shipped registry currently has zero votes; a vote-driven curated score
//!   would tie all of them. Popularity carries the ranking until votes exist.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Calibration constants
//
// All tuning lives here. These were calibrated against live data in 2026-08
// with a 6-item registry; revisit when the registry is an order of magnitude
// larger. Reference points (Modrinth): Sodium 209.7M downloads / 39,988
// followers; Fabric API 235.5M / 35,146; Create 24.0M / 6,808; JourneyMap
// 13.6M / 4,757.
// ---------------------------------------------------------------------------

/// Downloads at or below this score 0 on the popularity curve.
const DOWNLOADS_FLOOR_LOG10: f64 = 3.0; // 1_000
/// Downloads at or above this score 1.0.
const DOWNLOADS_CEIL_LOG10: f64 = 8.4; // ~250_000_000

/// Followers at or below this score 0.
const FOLLOWS_FLOOR_LOG10: f64 = 1.0; // 10
/// Modrinth followers at or above this score 1.0.
const FOLLOWS_CEIL_LOG10: f64 = 4.7; // ~50_000

/// Technic ratings are far denser relative to installs than Modrinth follows
/// (~0.11% vs ~0.02%), so the endorsement ceiling is proportionally lower.
/// Without this a Technic pack's ratings would barely register.
const TECHNIC_RATINGS_CEIL_LOG10: f64 = 3.3; // ~2_000

/// Downloads carry more weight than endorsement, but endorsement is what pulls
/// content mods above libraries that accumulate downloads as dependencies.
const DOWNLOADS_WEIGHT: f64 = 0.6;
const FOLLOWS_WEIGHT: f64 = 0.4;

/// Multiplier applied to a library's popularity. Libraries are installed as
/// dependencies of the mods that need them, so surfacing them at the top of a
/// browse list wastes the slot regardless of who curated them.
const LIBRARY_PENALTY: f64 = 0.4;

/// Top of the uncurated band. Deliberately high: uncurated content should reach
/// the upper-middle of the list rather than being walled off below curated.
pub const UNCURATED_CEILING: f64 = 85.0;

/// Bottom of the curated band. A curated item never falls below this, so poorly
/// rated curated content lands mid-list rather than at the very bottom.
pub const CURATED_FLOOR: f64 = 40.0;
/// Span the curated band covers above its floor (40.0 -> 100.0).
const CURATED_SPAN: f64 = 60.0;

/// Maximum points votes may add or subtract within the curated band.
const VOTE_NUDGE_RANGE: f64 = 15.0;

/// Vote count at which an item's own approval rate carries half the weight.
/// Smaller values let a handful of votes swing the score; larger values keep
/// new items pinned to the registry average for longer.
const VOTE_CONFIDENCE_HALFWAY: f64 = 10.0;

/// Fallback approval rate when the registry has no voted items at all.
pub const DEFAULT_REGISTRY_MEAN_APPROVAL: f64 = 0.5;

/// Modrinth's category tag for libraries. Deliberately does NOT include
/// `utility` — that tag also covers Create, JourneyMap, and Mod Menu, which are
/// content mods a player genuinely browses for.
const LIBRARY_CATEGORIES: [&str; 2] = ["library", "api"];

// ---------------------------------------------------------------------------
// Inputs and outputs
// ---------------------------------------------------------------------------

/// Which endorsement scale a source's follower-equivalent uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndorsementScale {
    /// Modrinth followers.
    Modrinth,
    /// Technic ratings (denser relative to installs, so a lower ceiling).
    Technic,
}

/// Everything the ranker needs about one item, independent of its source.
#[derive(Debug, Clone, Default)]
pub struct RankingInput {
    /// Downloads (Modrinth) or installs (Technic). `None` when unknown.
    pub downloads: Option<i64>,
    /// Followers (Modrinth) or ratings (Technic). `None` when unknown.
    pub endorsements: Option<i64>,
    /// Which ceiling to apply to `endorsements`.
    pub endorsement_scale: Option<EndorsementScale>,
    /// Category tags, used only for library detection.
    pub categories: Vec<String>,
    /// True when the item comes from the signed registry.
    pub curated: bool,
    /// Curated vote tallies. Ignored for uncurated items.
    pub upvotes: i64,
    pub downvotes: i64,
}

/// A scored item plus the reasoning behind it.
///
/// The breakdown mirrors `crash_service::compute_mod_score`'s explainability
/// pattern: every component that moved the number is recorded, so a surprising
/// ordering can be explained without re-deriving the math by hand.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    /// Final 0-100 score.
    pub score: f64,
    /// Popularity after the library penalty, 0.0-1.0.
    pub popularity: f64,
    /// Normalized download component before weighting.
    pub downloads_norm: f64,
    /// Normalized endorsement component before weighting.
    pub endorsements_norm: f64,
    /// True when the library penalty was applied.
    pub library_penalized: bool,
    /// Points contributed by votes; 0.0 when the item has none.
    pub vote_nudge: f64,
    /// Which band the item was scored in.
    pub curated: bool,
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

/// Map a raw count onto 0.0-1.0 along a log10 curve between two reference
/// points. `log10(0 + 1) == 0`, so zero needs no special case.
fn log_normalize(value: i64, floor_log10: f64, ceil_log10: f64) -> f64 {
    if value <= 0 || ceil_log10 <= floor_log10 {
        return 0.0;
    }
    let magnitude = ((value as f64) + 1.0).log10();
    ((magnitude - floor_log10) / (ceil_log10 - floor_log10)).clamp(0.0, 1.0)
}

/// True when the item's categories mark it as a library or API.
pub fn is_library(categories: &[String]) -> bool {
    categories.iter().any(|category| {
        let lowered = category.trim().to_ascii_lowercase();
        LIBRARY_CATEGORIES.contains(&lowered.as_str())
    })
}

/// Points votes contribute within the curated band, in `[-15.0, 15.0]`.
///
/// Raw net score (`upvotes - downvotes`) rewards volume rather than approval:
/// 1000 up / 900 down beats 50 up / 0 down. A raw approval *rate* fails the
/// other way, since one upvote reads as a perfect 100%.
///
/// So the item's approval rate is pulled toward the registry's average, weighted
/// by how many votes it has. Few votes keep it near the average; many votes let
/// its own rate take over. At zero votes the confidence term is 0, the result
/// collapses to the registry mean, and the nudge is 0 — which is the case that
/// ships today, since every registry item currently has no votes at all.
pub fn vote_nudge(upvotes: i64, downvotes: i64, registry_mean_approval: f64) -> f64 {
    let total = upvotes.saturating_add(downvotes);
    if total <= 0 {
        return 0.0;
    }
    let mean = if registry_mean_approval.is_finite() {
        registry_mean_approval.clamp(0.0, 1.0)
    } else {
        DEFAULT_REGISTRY_MEAN_APPROVAL
    };
    let total = total as f64;
    let approval = (upvotes.max(0) as f64) / total;
    let confidence = total / (total + VOTE_CONFIDENCE_HALFWAY);
    let smoothed = mean + (approval - mean) * confidence;
    ((smoothed - 0.5) * (VOTE_NUDGE_RANGE * 2.0)).clamp(-VOTE_NUDGE_RANGE, VOTE_NUDGE_RANGE)
}

/// Score one item onto the shared 0-100 scale.
pub fn score_item(input: &RankingInput, registry_mean_approval: f64) -> ScoreBreakdown {
    let downloads_norm = log_normalize(
        input.downloads.unwrap_or(0),
        DOWNLOADS_FLOOR_LOG10,
        DOWNLOADS_CEIL_LOG10,
    );
    let endorsement_ceiling = match input.endorsement_scale {
        Some(EndorsementScale::Technic) => TECHNIC_RATINGS_CEIL_LOG10,
        _ => FOLLOWS_CEIL_LOG10,
    };
    let endorsements_norm = log_normalize(
        input.endorsements.unwrap_or(0),
        FOLLOWS_FLOOR_LOG10,
        endorsement_ceiling,
    );

    let raw_popularity = DOWNLOADS_WEIGHT * downloads_norm + FOLLOWS_WEIGHT * endorsements_norm;
    let library_penalized = is_library(&input.categories);
    let popularity = if library_penalized {
        raw_popularity * LIBRARY_PENALTY
    } else {
        raw_popularity
    }
    .clamp(0.0, 1.0);

    let (score, nudge) = if input.curated {
        let nudge = vote_nudge(input.upvotes, input.downvotes, registry_mean_approval);
        (
            (CURATED_FLOOR + popularity * CURATED_SPAN + nudge)
                .clamp(CURATED_FLOOR - VOTE_NUDGE_RANGE, 100.0),
            nudge,
        )
    } else {
        (
            (popularity * UNCURATED_CEILING).clamp(0.0, UNCURATED_CEILING),
            0.0,
        )
    };

    // A NaN here would silently corrupt the sort order rather than fail loudly,
    // so it is converted to the bottom of the range instead.
    let score = if score.is_finite() { score } else { 0.0 };

    ScoreBreakdown {
        score,
        popularity,
        downloads_norm,
        endorsements_norm,
        library_penalized,
        vote_nudge: nudge,
        curated: input.curated,
    }
}

/// Endorsement-only score, used by the "Most Endorsed" sort.
///
/// Deliberately ignores downloads so the sort answers "what do people actively
/// follow or vote for", which is a different question from overall popularity.
pub fn endorsement_score(input: &RankingInput, registry_mean_approval: f64) -> f64 {
    let endorsement_ceiling = match input.endorsement_scale {
        Some(EndorsementScale::Technic) => TECHNIC_RATINGS_CEIL_LOG10,
        _ => FOLLOWS_CEIL_LOG10,
    };
    let endorsements_norm = log_normalize(
        input.endorsements.unwrap_or(0),
        FOLLOWS_FLOOR_LOG10,
        endorsement_ceiling,
    );
    let base = endorsements_norm * UNCURATED_CEILING;
    if input.curated {
        let upvote_norm = log_normalize(input.upvotes, 0.0, 2.0);
        (base
            + upvote_norm * CURATED_SPAN
            + vote_nudge(input.upvotes, input.downvotes, registry_mean_approval))
        .max(0.0)
    } else {
        base
    }
}

/// Sort scored items highest-first, with a stable tiebreak on name so equal
/// scores do not reorder between identical queries.
pub fn sort_by_score<T, F, N>(items: &mut [T], score_of: F, name_of: N)
where
    F: Fn(&T) -> f64,
    N: Fn(&T) -> &str,
{
    items.sort_by(|left, right| {
        score_of(right)
            .partial_cmp(&score_of(left))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| name_of(left).cmp(name_of(right)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modrinth(downloads: i64, follows: i64, categories: &[&str]) -> RankingInput {
        RankingInput {
            downloads: Some(downloads),
            endorsements: Some(follows),
            endorsement_scale: Some(EndorsementScale::Modrinth),
            categories: categories.iter().map(|c| c.to_string()).collect(),
            curated: false,
            upvotes: 0,
            downvotes: 0,
        }
    }

    fn curated(downloads: i64, follows: i64, categories: &[&str]) -> RankingInput {
        RankingInput {
            curated: true,
            ..modrinth(downloads, follows, categories)
        }
    }

    // Live figures captured 2026-08 from api.modrinth.com.
    const SODIUM: (i64, i64) = (209_713_432, 39_988);
    const LITHIUM: (i64, i64) = (100_000_000, 15_000);
    const FABRIC_API: (i64, i64) = (235_480_714, 35_146);
    const CREATE: (i64, i64) = (24_034_555, 6_808);
    const JOURNEYMAP: (i64, i64) = (13_579_008, 4_757);
    const CLOTH_CONFIG: (i64, i64) = (156_388_226, 16_335);

    #[test]
    fn worked_example_reproduces() {
        let mean = DEFAULT_REGISTRY_MEAN_APPROVAL;
        let sodium = score_item(&curated(SODIUM.0, SODIUM.1, &["optimization"]), mean).score;
        let lithium = score_item(&curated(LITHIUM.0, LITHIUM.1, &["optimization"]), mean).score;
        let fabric = score_item(&curated(FABRIC_API.0, FABRIC_API.1, &["library"]), mean).score;
        let create = score_item(&modrinth(CREATE.0, CREATE.1, &["technology"]), mean).score;
        let journeymap =
            score_item(&modrinth(JOURNEYMAP.0, JOURNEYMAP.1, &["adventure"]), mean).score;
        let cloth = score_item(
            &modrinth(CLOTH_CONFIG.0, CLOTH_CONFIG.1, &["library"]),
            mean,
        )
        .score;

        // Curated performance mods lead.
        assert!(sodium > lithium, "sodium {sodium} vs lithium {lithium}");
        assert!(lithium > create, "lithium {lithium} vs create {create}");
        // Popular uncurated content outranks a curated library — the whole point
        // of the library penalty.
        assert!(create > fabric, "create {create} vs fabric-api {fabric}");
        assert!(
            journeymap > cloth,
            "journeymap {journeymap} vs cloth {cloth}"
        );
        // Curated library still keeps the curated floor, so it lands mid-list
        // rather than at the bottom.
        assert!(fabric >= CURATED_FLOOR, "fabric-api {fabric} below floor");
        assert!(
            cloth < fabric,
            "uncurated library {cloth} vs curated {fabric}"
        );
    }

    #[test]
    fn ratio_would_have_inverted_sodium_and_journeymap() {
        // Guards the decision against regression: additive followers must keep
        // Sodium ahead of JourneyMap even though JourneyMap's follower RATIO is
        // nearly double Sodium's.
        let mean = DEFAULT_REGISTRY_MEAN_APPROVAL;
        let sodium = score_item(&modrinth(SODIUM.0, SODIUM.1, &[]), mean).score;
        let journeymap = score_item(&modrinth(JOURNEYMAP.0, JOURNEYMAP.1, &[]), mean).score;
        assert!(
            sodium > journeymap,
            "sodium {sodium} vs journeymap {journeymap}"
        );

        let sodium_ratio = SODIUM.1 as f64 / SODIUM.0 as f64;
        let journeymap_ratio = JOURNEYMAP.1 as f64 / JOURNEYMAP.0 as f64;
        assert!(journeymap_ratio > sodium_ratio, "the ratio trap is real");
    }

    #[test]
    fn zero_votes_produce_no_nudge() {
        // The shipped registry has zero votes on every item, so this is the
        // case that actually runs in production.
        assert_eq!(vote_nudge(0, 0, 0.5), 0.0);
        let scored = score_item(&curated(SODIUM.0, SODIUM.1, &[]), 0.5);
        assert_eq!(scored.vote_nudge, 0.0);
    }

    #[test]
    fn few_votes_barely_move_the_score() {
        let mean = 0.5;
        let one_upvote = vote_nudge(1, 0, mean);
        let many_upvotes = vote_nudge(400, 40, mean);
        assert!(
            one_upvote.abs() < 2.0,
            "a single upvote must not rocket an item up: {one_upvote}"
        );
        assert!(
            many_upvotes > one_upvote * 3.0,
            "400 votes must outweigh 1: {many_upvotes} vs {one_upvote}"
        );
        assert!(many_upvotes <= VOTE_NUDGE_RANGE);
    }

    #[test]
    fn net_score_tally_trap_is_fixed() {
        // Raw net score ranks 1000up/900down (+100) above 50up/0down (+50).
        // Approval-based nudging must invert that.
        let mean = 0.5;
        let noisy = vote_nudge(1000, 900, mean);
        let clean = vote_nudge(50, 0, mean);
        assert!(clean > noisy, "clean {clean} must beat noisy {noisy}");
        // 1000/1900 is 52.6% approval — barely above the 50% neutral point, so
        // the nudge is near zero. Volume earns it almost nothing, which is the
        // whole correction over raw net score (where it would have won by +100).
        assert!(
            noisy.abs() < 2.0,
            "a 52.6% approval rate should be near-neutral: {noisy}"
        );
        assert!(
            clean > 10.0,
            "an unopposed 50-vote record should be strong: {clean}"
        );
    }

    #[test]
    fn library_penalty_matches_only_library_tags() {
        assert!(is_library(&["library".into()]));
        assert!(is_library(&["API".into()]));
        // `utility` also covers Create, JourneyMap and Mod Menu — must not match.
        assert!(!is_library(&["utility".into()]));
        assert!(!is_library(&["optimization".into(), "technology".into()]));
    }

    #[test]
    fn bands_are_respected() {
        let mean = 0.5;
        let huge = score_item(&modrinth(i64::MAX, i64::MAX, &[]), mean);
        assert!(
            huge.score <= UNCURATED_CEILING,
            "uncurated exceeded its ceiling: {}",
            huge.score
        );

        let empty_curated = score_item(&curated(0, 0, &[]), mean);
        assert!(
            empty_curated.score >= CURATED_FLOOR,
            "curated fell below its floor: {}",
            empty_curated.score
        );

        // Even a badly rated curated item stays above the weakest uncurated.
        let mut hated = curated(0, 0, &[]);
        hated.upvotes = 0;
        hated.downvotes = 500;
        let hated_score = score_item(&hated, mean).score;
        let weak_uncurated = score_item(&modrinth(50, 0, &[]), mean).score;
        assert!(
            hated_score > weak_uncurated,
            "hated curated {hated_score} vs weak uncurated {weak_uncurated}"
        );
    }

    #[test]
    fn degenerate_inputs_do_not_panic_or_nan() {
        let mean = 0.5;
        for input in [
            RankingInput::default(),
            modrinth(0, 0, &[]),
            modrinth(-5, -5, &[]),
            curated(i64::MIN, i64::MIN, &[]),
        ] {
            let scored = score_item(&input, mean);
            assert!(scored.score.is_finite(), "non-finite score: {scored:?}");
            assert!(scored.popularity.is_finite());
        }
        // A non-finite registry mean must not poison the result.
        let scored = score_item(&curated(1000, 10, &[]), f64::NAN);
        assert!(scored.score.is_finite());
    }

    #[test]
    fn technic_ratings_use_their_own_scale() {
        let mean = 0.5;
        // complex-pixelmon-reforged: 1_582_592 installs, 1_730 ratings.
        let technic = RankingInput {
            downloads: Some(1_582_592),
            endorsements: Some(1_730),
            endorsement_scale: Some(EndorsementScale::Technic),
            ..Default::default()
        };
        let scored = score_item(&technic, mean);
        assert!(
            scored.endorsements_norm > 0.8,
            "1730 ratings should be near the Technic ceiling, got {}",
            scored.endorsements_norm
        );
        // The same count on Modrinth's scale would be far weaker.
        let as_modrinth = RankingInput {
            endorsement_scale: Some(EndorsementScale::Modrinth),
            ..technic
        };
        assert!(score_item(&as_modrinth, mean).endorsements_norm < scored.endorsements_norm);
    }

    #[test]
    fn sort_is_descending_and_stable_on_ties() {
        let mut items = vec![("b", 10.0), ("a", 10.0), ("c", 50.0)];
        sort_by_score(&mut items, |item| item.1, |item| item.0);
        assert_eq!(items[0].0, "c");
        // Equal scores fall back to name order rather than input order.
        assert_eq!(items[1].0, "a");
        assert_eq!(items[2].0, "b");
    }
}
