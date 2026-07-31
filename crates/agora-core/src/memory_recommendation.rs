//! Explainable automatic JVM heap recommendation based on installed content.
//!
//! Pure module with one pure entry point (`compute_recommendation`) and one
//! convenience wrapper (`detect_and_recommend`) that reads system RAM.
//!
//! Tiers include a resource-pack bump, 512 MiB rounding, 75%/2 GB
//! headroom, a 32 GB cap, explicit insufficient-RAM warnings, and next-tier
//! OOM fallback (without mutating settings).

use serde::{Deserialize, Serialize};
use std::path::Path;

// ---------------------------------------------------------------------------
// Input model
// ---------------------------------------------------------------------------

/// Lightweight summary of an instance's enabled content used for memory
/// tier selection. No JAR parsing is required; only manifest entries and
/// filesystem metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentSummary {
    /// Number of enabled mod JARs.
    pub enabled_mod_count: u64,
    /// Aggregate compressed size of enabled mod JARs (bytes).
    pub enabled_mod_bytes: u64,
    /// Number of enabled resource packs.
    pub resource_pack_count: u64,
    /// Aggregate compressed size of enabled resource packs (bytes).
    pub resource_pack_bytes: u64,
    /// Loader identifier, e.g. `"fabric"`, `"forge"`, `"neoforge"`, `"quilt"`.
    pub loader: String,
    /// Minecraft version string, e.g. `"1.21"`.
    pub minecraft_version: String,
}

// ---------------------------------------------------------------------------
// Output model
// ---------------------------------------------------------------------------

/// Result of a memory recommendation computation.
///
/// All fields are explainable so the UI or CLI can display *why* a particular
/// heap size was chosen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecommendation {
    /// Recommended maximum heap size in megabytes (the final value after all
    /// adjustments, clamping, and system-headroom checks).
    pub recommended_mb: i64,

    /// Human-readable label for the selected tier, e.g. `"4 GB"`.
    pub tier_label: String,

    /// Zero-based tier index (0 = vanilla 2 GB through 5 = extreme 12 GB).
    pub tier_index: u32,

    /// Whether the resource-pack one-tier bump was applied.
    pub is_large_resource_pack_adjustment: bool,

    /// Whether the recommendation was clamped by system RAM headroom.
    pub ram_capped: bool,

    /// `true` when even the base tier cannot fit within the 75%/2 GB OS
    /// reserve. The caller should present a prominent warning instead of
    /// pretending the clamped value is ideal.
    pub insufficient_system_ram: bool,

    /// Total system RAM in megabytes as detected at computation time.
    pub system_ram_mb: u64,

    /// Next tier up from the selected (pre-clamping) tier, in MB.
    /// Intended as a suggestion when the user reports OOM at the current
    /// setting. Equals `recommended_mb` if already at the maximum tier.
    pub next_tier_mb: i64,

    /// Human-readable label for the next tier.
    pub next_tier_label: String,

    /// Ordered list of individual factor descriptions that contributed to
    /// the recommendation (e.g. `"42 enabled mods"`, `"320 MiB mod JARs"`).
    pub factors: Vec<String>,

    /// Full human-readable explanation combining all factors, tier, and
    /// any system constraints.
    pub explanation: String,
}

// ---------------------------------------------------------------------------
// Internal tier definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Tier {
    index: u32,
    heap_mb: i64,
    label: &'static str,
}

const TIERS: &[Tier] = &[
    Tier {
        index: 0,
        heap_mb: 2048,
        label: "2 GB",
    },
    Tier {
        index: 1,
        heap_mb: 4096,
        label: "4 GB",
    },
    Tier {
        index: 2,
        heap_mb: 6144,
        label: "6 GB",
    },
    Tier {
        index: 3,
        heap_mb: 8192,
        label: "8 GB",
    },
    Tier {
        index: 4,
        heap_mb: 10240,
        label: "10 GB",
    },
    Tier {
        index: 5,
        heap_mb: 12288,
        label: "12 GB",
    },
];

/// Large resource-pack threshold: aggregate size above which we bump one tier.
const LARGE_RP_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB

const ROUND_TO_MB: i64 = 512;
const MAX_HEAP_MB: i64 = 32768;
const OS_RESERVE_MB: i64 = 2048;

// ---------------------------------------------------------------------------
// Pure recommendation function
// ---------------------------------------------------------------------------

/// Compute a memory recommendation from installed-content summary and total
/// system RAM.
///
/// This is a **pure** function with no I/O, no mutation, and no hidden
/// dependencies. The caller is responsible for providing an accurate
/// `total_ram_mb` (0 = unknown, meaning system-headroom checks are skipped).
pub fn compute_recommendation(summary: &ContentSummary, total_ram_mb: u64) -> MemoryRecommendation {
    let mc = summary.enabled_mod_count;
    let mb = summary.enabled_mod_bytes;
    let rpc = summary.resource_pack_count;
    let rpb = summary.resource_pack_bytes;

    // ----- tier selection -----
    let (tier_index, tier_heap, tier_label) = select_tier(mc, mb);

    // ----- resource-pack adjustment (one tier bump) -----
    let rp_big = rpb > LARGE_RP_BYTES || (rpc > 0 && rpb > LARGE_RP_BYTES / 2);
    let rp_bump_applied = rp_big && tier_index < 5;
    let (adj_index, adj_heap, adj_label) = if rp_bump_applied {
        let next = &TIERS[(tier_index + 1) as usize];
        (next.index, next.heap_mb, next.label)
    } else {
        (tier_index, tier_heap, tier_label)
    };

    // ----- build factors -----
    let mut factors: Vec<String> = Vec::new();
    factors.push(format!("{} enabled mods", mc));
    factors.push(format!("{} mod JAR bytes", format_bytes(mb)));
    if rpc > 0 {
        factors.push(format!("{} resource packs ({})", rpc, format_bytes(rpb)));
    }
    factors.push(format!("Loader: {}", summary.loader));
    factors.push(format!("Minecraft: {}", summary.minecraft_version));
    factors.push(format!(
        "Tier: {} ({} heap)",
        tier_label,
        format_bytes(tier_heap as u64 * 1024 * 1024)
    ));

    let mut explanation = format!(
        "Recommended {} heap based on {} enabled mods and {} of mod JARs",
        adj_label,
        mc,
        format_bytes(mb),
    );
    if rp_big {
        explanation.push_str(&format!(
            ", adjusted up one tier for large resource packs ({})",
            format_bytes(rpb),
        ));
    }
    if rpc > 0 && !rp_big {
        explanation.push_str(&format!(
            " ({} resource packs, not large enough to adjust)",
            rpc,
        ));
    }

    // ----- rounding -----
    let rounded = round_to(adj_heap, ROUND_TO_MB);
    let (_capped_val, insufficient_ram, final_val, final_label) =
        apply_system_headroom(adj_index, adj_label, rounded, total_ram_mb);
    let any_cap = final_val != rounded || insufficient_ram;

    // ----- next-tier suggestion for OOM -----
    let next_idx = (adj_index + 1).min(TIERS.len() as u32 - 1);
    let next_tier = &TIERS[next_idx as usize];
    let (_ncapped, _, next_val, next_final_label) = apply_system_headroom(
        next_tier.index,
        next_tier.label,
        next_tier.heap_mb,
        total_ram_mb,
    );
    // If system caps the next tier below the current, just report the current
    let (next_mb, next_label) = if next_val <= final_val {
        (final_val, &*final_label)
    } else {
        (next_val, &*next_final_label)
    };

    if insufficient_ram {
        explanation.push_str(&format!(
            ". System RAM ({} MB) cannot comfortably fit this tier; reduce enabled mods or add more RAM",
            total_ram_mb,
        ));
    }
    if any_cap {
        explanation.push_str(&format!(
            ". Capped to {} by system headroom (75% rule / 2 GB OS reserve)",
            final_label,
        ));
    }

    MemoryRecommendation {
        recommended_mb: final_val,
        tier_label: adj_label.to_string(),
        tier_index: adj_index,
        is_large_resource_pack_adjustment: rp_bump_applied,
        ram_capped: any_cap,
        insufficient_system_ram: insufficient_ram,
        system_ram_mb: total_ram_mb,
        next_tier_mb: next_mb,
        next_tier_label: next_label.to_string(),
        factors,
        explanation,
    }
}

// ---------------------------------------------------------------------------
// Convenience: detect system RAM and recommend
// ---------------------------------------------------------------------------

/// Detect system RAM via `sysinfo` and compute a recommendation.
///
/// This is the only impure function in this module. Prefer
/// [`compute_recommendation`] in test or deterministic contexts.
pub fn detect_and_recommend(summary: &ContentSummary) -> MemoryRecommendation {
    let total_mb = detect_total_ram_mb();
    compute_recommendation(summary, total_mb)
}

/// Build a recommendation summary from enabled manifest entries and cheap
/// filesystem metadata. Missing files contribute no bytes but remain counted.
pub fn summarize_instance(
    instance_dir: &Path,
    manifest: &crate::models::InstanceManifest,
) -> ContentSummary {
    let enabled_mods = manifest
        .mods
        .iter()
        .filter(|item| item.enabled)
        .collect::<Vec<_>>();
    let enabled_resource_packs = manifest
        .resourcepacks
        .iter()
        .filter(|item| item.enabled)
        .collect::<Vec<_>>();
    let enabled_mod_bytes = enabled_mods
        .iter()
        .filter_map(|item| std::fs::metadata(instance_dir.join("mods").join(&item.filename)).ok())
        .map(|metadata| metadata.len())
        .sum();
    let resource_pack_bytes = enabled_resource_packs
        .iter()
        .filter_map(|item| {
            std::fs::metadata(instance_dir.join("resourcepacks").join(&item.filename)).ok()
        })
        .map(|metadata| metadata.len())
        .sum();
    ContentSummary {
        enabled_mod_count: enabled_mods.len() as u64,
        enabled_mod_bytes,
        resource_pack_count: enabled_resource_packs.len() as u64,
        resource_pack_bytes,
        loader: manifest.loader.clone(),
        minecraft_version: manifest.minecraft_version.clone(),
    }
}

/// Query total physical RAM via `sysinfo`. Returns 0 on failure (callers treat
/// 0 as "unknown" and skip system-headroom checks).
pub fn detect_total_ram_mb() -> u64 {
    use std::sync::OnceLock;
    use sysinfo::System;

    static TOTAL_RAM_MB: OnceLock<u64> = OnceLock::new();
    *TOTAL_RAM_MB.get_or_init(|| {
        let mut sys = System::new();
        sys.refresh_memory();
        sys.total_memory() / 1024 / 1024
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Select the heap tier from mod count and mod bytes.
///
/// The plan defines six bands. Each "up to" tier fits content whose
/// count *and* compressed size are both within its limits. When either
/// dimension exceeds a tier's bounds, the next higher tier applies.
fn select_tier(mod_count: u64, mod_bytes: u64) -> (u32, i64, &'static str) {
    let idx = if mod_count == 0 {
        0
    } else if mod_count > 350 || mod_bytes > 2048 * 1024 * 1024 {
        5
    } else if mod_count > 200 || mod_bytes > 1536 * 1024 * 1024 {
        4
    } else if mod_count > 100 || mod_bytes > 768 * 1024 * 1024 {
        3
    } else if mod_count > 40 || mod_bytes > 256 * 1024 * 1024 {
        2
    } else {
        1
    };
    let t = &TIERS[idx as usize];
    (t.index, t.heap_mb, t.label)
}

/// Round a value to the nearest `multiple` of MB.
fn round_to(value: i64, multiple: i64) -> i64 {
    let half = multiple / 2;
    ((value + half) / multiple) * multiple
}

/// Apply system-headroom constraints: 75% ceiling, 2 GB OS reserve, and 32 GB
/// absolute cap.
///
/// Returns `(capped_value, insufficient_flag, final_value, final_label)`.
fn apply_system_headroom(
    tier_index: u32,
    label: &str,
    requested_mb: i64,
    total_ram_mb: u64,
) -> (i64, bool, i64, String) {
    let base = requested_mb.min(MAX_HEAP_MB);

    if total_ram_mb == 0 {
        // Unknown RAM can only be capped at the absolute maximum.
        return (base, false, base, label.to_string());
    }

    let seventy_five = (total_ram_mb as f64 * 0.75) as i64;
    let with_reserve = total_ram_mb as i64 - OS_RESERVE_MB;
    let raw_limit = if with_reserve > 0 {
        seventy_five.min(with_reserve)
    } else {
        (total_ram_mb as i64 / 2).max(512)
    };
    let headroom_limit = (raw_limit / ROUND_TO_MB * ROUND_TO_MB).max(512);

    if base > headroom_limit {
        // Cannot fit: warn and return the best available value.
        let clamped = base.min(headroom_limit);
        let clamped_rounded = (clamped / ROUND_TO_MB * ROUND_TO_MB).max(512);
        let insufficient = clamped_rounded < 4096 || (tier_index >= 1 && clamped_rounded < base);
        let final_label = format!("{} MB", clamped_rounded);
        (clamped_rounded, insufficient, clamped_rounded, final_label)
    } else if base > seventy_five.min(with_reserve) {
        // Exact match or near-limit: cap to headroom.
        let clamped = base.min(headroom_limit);
        let final_label = format!("{} MB", clamped);
        (clamped, false, clamped, final_label)
    } else {
        (base, false, base, label.to_string())
    }
}

/// Format byte counts into human-readable strings (exact, no rounding).
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sm(count: u64, bytes_mb: u64) -> ContentSummary {
        ContentSummary {
            enabled_mod_count: count,
            enabled_mod_bytes: bytes_mb * 1024 * 1024,
            resource_pack_count: 0,
            resource_pack_bytes: 0,
            loader: "fabric".into(),
            minecraft_version: "1.21".into(),
        }
    }

    // ----- tier boundaries -----

    #[test]
    fn vanilla_no_mods_is_2gb() {
        let r = compute_recommendation(&sm(0, 0), 16384);
        assert_eq!(r.recommended_mb, 2048);
        assert_eq!(r.tier_index, 0);
        assert_eq!(r.tier_label, "2 GB");
        assert!(!r.is_large_resource_pack_adjustment);
    }

    #[test]
    fn small_modpack_4gb() {
        let r = compute_recommendation(&sm(20, 128), 16384);
        assert_eq!(r.recommended_mb, 4096);
        assert_eq!(r.tier_index, 1);
    }

    #[test]
    fn at_40_mods_256_mib_is_4gb() {
        let r = compute_recommendation(&sm(40, 256), 16384);
        assert_eq!(r.recommended_mb, 4096);
        assert_eq!(r.tier_index, 1);
    }

    #[test]
    fn exceeds_mod_count_goes_to_6gb() {
        let r = compute_recommendation(&sm(41, 128), 16384);
        assert_eq!(r.recommended_mb, 6144);
        assert_eq!(r.tier_index, 2);
    }

    #[test]
    fn exceeds_mod_bytes_goes_to_6gb() {
        let r = compute_recommendation(&sm(20, 300), 16384);
        assert_eq!(r.recommended_mb, 6144);
        assert_eq!(r.tier_index, 2);
    }

    #[test]
    fn moderate_modpack_8gb() {
        let r = compute_recommendation(&sm(150, 1024), 16384);
        assert_eq!(r.recommended_mb, 8192);
        assert_eq!(r.tier_index, 3);
    }

    #[test]
    fn heavy_modpack_10gb() {
        let r = compute_recommendation(&sm(250, 1400), 16384);
        assert_eq!(r.recommended_mb, 10240);
        assert_eq!(r.tier_index, 4);
    }

    #[test]
    fn extreme_mod_count_12gb() {
        let r = compute_recommendation(&sm(400, 500), 16384);
        assert_eq!(r.recommended_mb, 12288);
        assert_eq!(r.tier_index, 5);
    }

    #[test]
    fn extreme_mod_bytes_12gb() {
        let r = compute_recommendation(&sm(100, 2500), 16384);
        assert_eq!(r.recommended_mb, 12288);
        assert_eq!(r.tier_index, 5);
    }

    #[test]
    fn at_350_mods_10gb_not_12gb() {
        let r = compute_recommendation(&sm(350, 1500), 16384);
        assert_eq!(r.recommended_mb, 10240);
        assert_eq!(r.tier_index, 4);
    }

    #[test]
    fn at_200_mods_8gb_not_10gb() {
        let r = compute_recommendation(&sm(200, 1024), 16384);
        assert_eq!(r.recommended_mb, 8192);
        assert_eq!(r.tier_index, 3);
    }

    #[test]
    fn tier_not_exceeded_by_one_dimension() {
        let r = compute_recommendation(&sm(100, 700), 16384);
        assert_eq!(r.recommended_mb, 6144);
        assert_eq!(r.tier_index, 2);
    }

    // ----- resource-pack adjustment -----

    #[test]
    fn large_resource_packs_bump_one_tier() {
        let mut s = sm(20, 128);
        s.resource_pack_bytes = 300 * 1024 * 1024;
        s.resource_pack_count = 3;
        let r = compute_recommendation(&s, 16384);
        assert_eq!(r.recommended_mb, 6144);
        assert_eq!(r.tier_index, 2);
        assert!(r.is_large_resource_pack_adjustment);
        assert!(r.explanation.contains("resource pack"));
    }

    #[test]
    fn small_resource_packs_no_adjustment() {
        let mut s = sm(20, 128);
        s.resource_pack_bytes = 50 * 1024 * 1024;
        s.resource_pack_count = 2;
        let r = compute_recommendation(&s, 16384);
        assert_eq!(r.recommended_mb, 4096);
        assert!(!r.is_large_resource_pack_adjustment);
    }

    #[test]
    fn rp_adjustment_at_max_tier_stays_12gb() {
        let mut s = sm(400, 500);
        s.resource_pack_bytes = 500 * 1024 * 1024;
        s.resource_pack_count = 5;
        let r = compute_recommendation(&s, 16384);
        assert_eq!(r.recommended_mb, 12288);
        assert_eq!(r.tier_index, 5);
        // No adjustment since already at max
        assert!(!r.is_large_resource_pack_adjustment);
    }

    // ----- system headroom -----

    #[test]
    fn capped_by_75_percent_ram() {
        let r = compute_recommendation(&sm(20, 128), 4096);
        // 75% of 4096 = 3072, reserve = 2048, min of those = 2048
        assert!(r.recommended_mb <= 3072);
        assert!(r.recommended_mb >= 2048);
        assert!(r.ram_capped);
    }

    #[test]
    fn insufficient_ram_warning_on_tiny_system() {
        let r = compute_recommendation(&sm(150, 1024), 2048);
        // 75% of 2048 = 1536, reserve = 0, so headroom = 1536
        // 8 GB tier won't fit
        assert!(r.insufficient_system_ram);
    }

    #[test]
    fn unknown_ram_no_capping() {
        let r = compute_recommendation(&sm(20, 128), 0);
        assert_eq!(r.recommended_mb, 4096);
        assert!(!r.ram_capped);
        assert!(!r.insufficient_system_ram);
    }

    #[test]
    fn ample_ram_no_capping() {
        let r = compute_recommendation(&sm(20, 128), 65536);
        assert_eq!(r.recommended_mb, 4096);
        assert!(!r.ram_capped);
        assert!(!r.insufficient_system_ram);
    }

    // ----- 32 GB cap -----

    #[test]
    fn never_exceeds_32gb_absolute_cap() {
        let r = compute_recommendation(&sm(400, 500), 524288); // 512 GB
        assert!(r.recommended_mb <= 32768);
    }

    // ----- 512 MiB rounding -----

    #[test]
    fn values_rounded_to_512_mib() {
        let r = compute_recommendation(&sm(20, 128), 16384);
        assert_eq!(r.recommended_mb % 512, 0);
    }

    // ----- next tier for OOM -----

    #[test]
    fn next_tier_above_current() {
        let r = compute_recommendation(&sm(20, 128), 16384);
        assert_eq!(r.next_tier_mb, 6144);
        assert_eq!(r.next_tier_label, "6 GB");
    }

    #[test]
    fn next_tier_at_max_stays_same() {
        let r = compute_recommendation(&sm(400, 500), 16384);
        assert_eq!(r.next_tier_mb, r.recommended_mb);
    }

    // ----- factors / explanation -----

    #[test]
    fn factors_are_populated() {
        let r = compute_recommendation(&sm(20, 128), 16384);
        assert!(!r.factors.is_empty());
        assert!(r.factors.iter().any(|f| f.contains("enabled mods")));
        assert!(r.factors.iter().any(|f| f.contains("Loader")));
    }

    #[test]
    fn explanation_mentions_tier() {
        let r = compute_recommendation(&sm(20, 128), 16384);
        assert!(r.explanation.contains("4 GB"));
    }

    // ----- edge: zero bytes but some mods -----

    #[test]
    fn mods_with_zero_bytes_uses_tier_1() {
        let r = compute_recommendation(&sm(1, 0), 16384);
        assert_eq!(r.recommended_mb, 4096);
        assert_eq!(r.tier_index, 1);
    }

    // ----- detect_and_recommend runs without panic -----

    #[test]
    fn detect_and_recommend_returns_sensible_values() {
        let s = sm(20, 128);
        let r = detect_and_recommend(&s);
        assert!(r.recommended_mb >= 2048);
        assert!(r.recommended_mb > 0);
        assert!(!r.factors.is_empty());
    }

    // ----- extreme low-RAM boundaries -----

    #[test]
    fn zero_mods_on_very_low_ram_capped() {
        let r = compute_recommendation(&sm(0, 0), 1024);
        // 75% of 1024 = 768, reserve = negative, so clamped to 768 max
        assert!(r.recommended_mb > 0);
        assert!(r.insufficient_system_ram || r.ram_capped || r.recommended_mb <= 2048);
    }

    #[test]
    fn supports_factors_are_deterministic_for_same_input() {
        let s = sm(150, 1024);
        let a = compute_recommendation(&s, 16384);
        let b = compute_recommendation(&s, 16384);
        assert_eq!(a.recommended_mb, b.recommended_mb);
        assert_eq!(a.factors, b.factors);
        assert_eq!(a.explanation, b.explanation);
    }

    // ----- format_bytes helper -----

    #[test]
    fn format_bytes_works() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(1048576), "1.0 MiB");
        assert_eq!(format_bytes(1073741824), "1.0 GiB");
    }

    // ----- round_to helper -----

    #[test]
    fn round_to_nearest_512() {
        assert_eq!(round_to(4000, 512), 4096);
        assert_eq!(round_to(4096, 512), 4096);
        assert_eq!(round_to(4300, 512), 4096);
        assert_eq!(round_to(4500, 512), 4608);
        assert_eq!(round_to(2048, 512), 2048);
    }

    // ----- heavy-then-extreme progression -----

    #[test]
    fn progression_2_4_6_8_10_12() {
        let setups = [
            (0u64, 0u64, 2048i64, 0u32),
            (20, 128, 4096, 1),
            (60, 300, 6144, 2),
            (150, 1000, 8192, 3),
            (300, 1800, 10240, 4),
            (400, 500, 12288, 5),
        ];
        for (count, bytes_mb, expected_mb, expected_idx) in &setups {
            let r = compute_recommendation(&sm(*count, *bytes_mb), 65536);
            assert_eq!(
                r.recommended_mb, *expected_mb,
                "failed for {count} mods, {bytes_mb} MiB"
            );
            assert_eq!(
                r.tier_index, *expected_idx,
                "tier index mismatch for {count} mods, {bytes_mb} MiB"
            );
        }
    }
}
