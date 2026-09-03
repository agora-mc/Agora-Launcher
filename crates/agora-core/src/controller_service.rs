//! Controller support policy: when to offer Controlify, and which instances
//! the user has already said no to.
//!
//! Agora can tell that a gamepad is connected — the frontend reads the Web
//! Gamepad API — but a connected gamepad does *not* mean Minecraft can use it.
//! Vanilla Minecraft has no controller support at all, so a user who launches
//! from a couch with a pad in hand lands in a game they cannot play. Controlify
//! is the mod that fixes that, and noticing the gap on the user's behalf is the
//! whole point of handheld mode.
//!
//! This module owns the *decision*; detection is a platform mechanism and lives
//! in the adapter. Everything here is pure apart from the two functions that
//! read and write the decline list through [`SettingsService`].

use std::collections::BTreeSet;

use crate::error::LauncherResult;
use crate::models::InstanceManifest;
use crate::settings::SettingsService;

/// Controlify's loader mod id, as it appears in `fabric.mod.json` /
/// `mods.toml`. Used to detect an existing install.
pub const CONTROLIFY_MOD_ID: &str = "controlify";

/// Controlify's Modrinth slug.
///
/// Modrinth's `/v2/project/{id|slug}` endpoint resolves slugs as well as ids,
/// so the install path can take this verbatim. A slug is used rather than the
/// opaque project id because it is human-checkable: a reviewer can open
/// `modrinth.com/mod/controlify` and confirm it, which an id does not allow.
pub const CONTROLIFY_MODRINTH_SLUG: &str = "controlify";

/// Settings key holding the instances the user declined the offer for.
pub const CONTROLIFY_DECLINED_KEY: &str = "controlify_offer_declined";

/// Settings key for whether handheld/big-picture mode may be entered at all.
pub const CONTROLLER_MODE_ENABLED_KEY: &str = "controller_mode_enabled";

/// Settings key for entering handheld mode automatically on gamepad connect.
pub const CONTROLLER_MODE_AUTO_KEY: &str = "controller_mode_auto_enter";

/// Loaders Controlify is published for.
///
/// Deliberately conservative. Offering a mod that then fails to resolve looks
/// like a broken launcher; staying quiet on a loader it might support only
/// costs one un-made suggestion. Widen this when a build is confirmed, not
/// when one seems likely.
const CONTROLIFY_LOADERS: [&str; 3] = ["fabric", "quilt", "neoforge"];

/// Why the offer is or is not being made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlifyOfferDecision {
    /// Worth asking: a controller is in play, the loader is supported, and
    /// Controlify is not installed.
    Offer,
    /// Already present. Nothing to do — including when it is installed but
    /// disabled, because re-offering an install would not fix that and the
    /// user disabled it on purpose.
    AlreadyInstalled,
    /// Controlify has no build for this instance's loader.
    UnsupportedLoader,
    /// The user said no for this instance.
    Declined,
    /// The instance is locked, so nothing may be installed into it.
    InstanceLocked,
}

impl ControlifyOfferDecision {
    /// Whether the UI should show anything at all.
    pub fn should_prompt(self) -> bool {
        matches!(self, Self::Offer)
    }
}

/// The result of evaluating the Controlify offer for one instance.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ControlifyOffer {
    pub instance_id: String,
    pub decision: ControlifyOfferDecision,
    /// The Modrinth slug to install, present only when `decision` is `Offer`
    /// so a caller cannot accidentally install off a negative answer.
    pub modrinth_slug: Option<String>,
    /// Short explanation, suitable for a tooltip or a log line.
    pub reason: String,
}

/// Decide whether to offer Controlify for `manifest`.
///
/// `declined` is the caller's answer to "has the user already refused for this
/// instance", normally from [`controlify_declined_instances`]. It is a
/// parameter rather than a lookup so this stays pure and testable.
pub fn evaluate_controlify_offer(manifest: &InstanceManifest, declined: bool) -> ControlifyOffer {
    let instance_id = manifest.instance_id.clone();
    let decide = |decision, reason: &str| ControlifyOffer {
        instance_id: instance_id.clone(),
        decision,
        modrinth_slug: match decision {
            ControlifyOfferDecision::Offer => Some(CONTROLIFY_MODRINTH_SLUG.to_string()),
            _ => None,
        },
        reason: reason.to_string(),
    };

    // Order matters only for the message the user sees; each check is
    // independent. "Already installed" comes first because it is the answer
    // that stays true no matter what else is going on.
    if has_controlify(manifest) {
        return decide(
            ControlifyOfferDecision::AlreadyInstalled,
            "Controlify is already installed in this instance.",
        );
    }
    if manifest.is_locked {
        return decide(
            ControlifyOfferDecision::InstanceLocked,
            "This instance is locked, so no mods can be added to it.",
        );
    }
    if !loader_supported(&manifest.loader) {
        return decide(
            ControlifyOfferDecision::UnsupportedLoader,
            "Controlify has no build for this instance's mod loader.",
        );
    }
    if declined {
        return decide(
            ControlifyOfferDecision::Declined,
            "You chose not to add Controlify to this instance.",
        );
    }
    decide(
        ControlifyOfferDecision::Offer,
        "Minecraft has no built-in controller support. Controlify adds it.",
    )
}

/// True when the instance's loader has a Controlify build.
fn loader_supported(loader: &str) -> bool {
    let loader = loader.trim().to_ascii_lowercase();
    CONTROLIFY_LOADERS.contains(&loader.as_str())
}

/// True when Controlify is present in the manifest, enabled or not.
///
/// Checks every identity channel an install could have landed under: the
/// loader id from the jar, the curated registry id, and the filename. A mod
/// dropped in by hand counts — the question is "does this instance already
/// have controller support", not "did Agora install it".
fn has_controlify(manifest: &InstanceManifest) -> bool {
    manifest.mods.iter().any(|m| {
        let jar_id_matches = m
            .mod_jar_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(CONTROLIFY_MOD_ID));
        let provides_it = m
            .provided_mod_ids
            .iter()
            .any(|id| id.eq_ignore_ascii_case(CONTROLIFY_MOD_ID));
        let registry_id_matches = m
            .registry_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case(CONTROLIFY_MOD_ID));
        let filename_matches = m
            .filename
            .to_ascii_lowercase()
            .starts_with(CONTROLIFY_MOD_ID);
        jar_id_matches || provides_it || registry_id_matches || filename_matches
    })
}

/// Instances the user has declined the Controlify offer for.
///
/// Stored as one JSON array under a single settings key rather than a row per
/// instance: the list is bounded by the instance count and is only ever read
/// whole. A malformed or missing value reads as "nobody declined", which
/// re-asks rather than silently suppressing the offer — the failure the user
/// can act on is the better one.
pub fn controlify_declined_instances(settings: &SettingsService) -> BTreeSet<String> {
    settings
        .get(CONTROLIFY_DECLINED_KEY)
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
        .map(|ids| ids.into_iter().filter(|id| !id.is_empty()).collect())
        .unwrap_or_default()
}

/// Record that the user declined the offer for `instance_id`.
///
/// Idempotent, and preserves any other instance already in the list.
pub fn decline_controlify_for(settings: &SettingsService, instance_id: &str) -> LauncherResult<()> {
    let mut declined = controlify_declined_instances(settings);
    declined.insert(instance_id.to_string());
    let ids: Vec<String> = declined.into_iter().collect();
    settings.set(CONTROLIFY_DECLINED_KEY, &serde_json::json!(ids))
}

/// Forget a previous decline, so the offer can be made again.
///
/// The user needs a way back: declining once should not permanently hide a
/// feature, and there is no other route to un-decline from the UI.
pub fn reset_controlify_decline(
    settings: &SettingsService,
    instance_id: &str,
) -> LauncherResult<()> {
    let mut declined = controlify_declined_instances(settings);
    if !declined.remove(instance_id) {
        return Ok(());
    }
    let ids: Vec<String> = declined.into_iter().collect();
    settings.set(CONTROLIFY_DECLINED_KEY, &serde_json::json!(ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InstalledMod, InstanceManifest};

    fn manifest_with(loader: &str, mods: Vec<InstalledMod>) -> InstanceManifest {
        InstanceManifest {
            manifest_version: crate::models::CURRENT_MANIFEST_VERSION,
            pack_origin: None,
            instance_id: "inst-1".into(),
            name: "Test".into(),
            created_from_pack: None,
            minecraft_version: "1.21".into(),
            loader: loader.into(),
            loader_version: "0.16.0".into(),
            is_locked: false,
            mods,
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::json!({}),
        }
    }

    fn a_mod(filename: &str, jar_id: Option<&str>) -> InstalledMod {
        InstalledMod {
            filename: filename.into(),
            registry_id: None,
            modrinth_id: None,
            source: "manual".into(),
            source_url: None,
            version: None,
            sha256: String::new(),
            installed_at: String::new(),
            java_packages: vec![],
            mod_jar_id: jar_id.map(str::to_string),
            provided_mod_ids: vec![],
            enabled: true,
            content_type: "mod".into(),
            update_pinned: false,
            pack_managed: false,
            installed_as_dependency: false,
            depends_on: vec![],
            optional_deps: vec![],
            incompatible_deps: vec![],
        }
    }

    #[test]
    fn offers_on_a_supported_loader_without_controlify() {
        let manifest = manifest_with("fabric", vec![a_mod("sodium.jar", Some("sodium"))]);
        let offer = evaluate_controlify_offer(&manifest, false);
        assert_eq!(offer.decision, ControlifyOfferDecision::Offer);
        assert!(offer.decision.should_prompt());
        assert_eq!(
            offer.modrinth_slug.as_deref(),
            Some(CONTROLIFY_MODRINTH_SLUG)
        );
    }

    #[test]
    fn every_negative_decision_withholds_the_slug() {
        // A caller that ignores `decision` must not be able to install anyway.
        let locked = {
            let mut m = manifest_with("fabric", vec![]);
            m.is_locked = true;
            m
        };
        for manifest in [
            manifest_with("forge", vec![]),
            manifest_with("fabric", vec![a_mod("controlify-2.0.jar", None)]),
            locked,
        ] {
            let offer = evaluate_controlify_offer(&manifest, false);
            assert!(!offer.decision.should_prompt());
            assert!(offer.modrinth_slug.is_none());
        }
        let declined = evaluate_controlify_offer(&manifest_with("fabric", vec![]), true);
        assert_eq!(declined.decision, ControlifyOfferDecision::Declined);
        assert!(declined.modrinth_slug.is_none());
    }

    #[test]
    fn unsupported_loader_is_not_offered() {
        let manifest = manifest_with("forge", vec![]);
        assert_eq!(
            evaluate_controlify_offer(&manifest, false).decision,
            ControlifyOfferDecision::UnsupportedLoader
        );
    }

    #[test]
    fn supported_loaders_are_matched_case_insensitively() {
        for loader in ["Fabric", "QUILT", " neoforge "] {
            let manifest = manifest_with(loader, vec![]);
            assert_eq!(
                evaluate_controlify_offer(&manifest, false).decision,
                ControlifyOfferDecision::Offer,
                "expected {loader} to be supported"
            );
        }
    }

    #[test]
    fn detects_controlify_through_every_identity_channel() {
        let by_jar_id = a_mod("something.jar", Some("controlify"));
        let by_filename = a_mod("Controlify-2.0.4+1.21.jar", None);
        let by_registry_id = {
            let mut m = a_mod("cf.jar", None);
            m.registry_id = Some("Controlify".into());
            m
        };
        let by_provides = {
            let mut m = a_mod("bundle.jar", Some("bundle"));
            m.provided_mod_ids = vec!["controlify".into()];
            m
        };
        for installed in [by_jar_id, by_filename, by_registry_id, by_provides] {
            let manifest = manifest_with("fabric", vec![installed.clone()]);
            assert_eq!(
                evaluate_controlify_offer(&manifest, false).decision,
                ControlifyOfferDecision::AlreadyInstalled,
                "missed Controlify in {}",
                installed.filename
            );
        }
    }

    #[test]
    fn a_disabled_controlify_still_counts_as_installed() {
        // Re-offering an install would not enable it, and the user disabled it
        // deliberately. Nagging here would be wrong.
        let mut disabled = a_mod("controlify.jar", Some("controlify"));
        disabled.enabled = false;
        let manifest = manifest_with("fabric", vec![disabled]);
        assert_eq!(
            evaluate_controlify_offer(&manifest, false).decision,
            ControlifyOfferDecision::AlreadyInstalled
        );
    }

    #[test]
    fn a_mod_merely_starting_with_control_is_not_controlify() {
        let manifest = manifest_with("fabric", vec![a_mod("controlling-12.0.jar", None)]);
        assert_eq!(
            evaluate_controlify_offer(&manifest, false).decision,
            ControlifyOfferDecision::Offer
        );
    }

    #[test]
    fn locked_instance_outranks_a_supported_loader() {
        let mut manifest = manifest_with("fabric", vec![]);
        manifest.is_locked = true;
        assert_eq!(
            evaluate_controlify_offer(&manifest, false).decision,
            ControlifyOfferDecision::InstanceLocked
        );
    }
}
