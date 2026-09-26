//! Content providers, end to end, against the runtime that actually ships.
//!
//! The shipped example provider is installed from `examples/plugins/provider`
//! and driven through `PluginService`, the real `QuickJsHost`, the provider
//! registry, Browse, and install resolution. The point is the claim the whole
//! design rests on: a community provider reaches Browse and the install
//! pipeline through exactly the interface Agora's own providers use, and every
//! answer it gives is checked by core rather than trusted.

use agora_core::ctx::Ctx;
use agora_core::models::InstanceManifest;
use agora_core::plugins::{PluginService, PLUGINS_ENABLED_SETTING};
use agora_core::providers::{
    browse, install, ProviderOrigin, ProviderRegistry, ResolveRequest, SearchRequest,
    VersionsRequest,
};
use agora_plugin_api::host::ScriptHost;
use agora_plugin_api::provider::InstallPlan;
use agora_plugin_host::QuickJsHost;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const PROVIDER: &str = "agora.example-provider/shelf";

struct World {
    dir: tempfile::TempDir,
    ctx: Ctx,
    service: PluginService,
}

fn world() -> World {
    let dir = tempfile::tempdir().expect("temp dir");
    let ctx = Ctx::for_testing(dir.path().to_path_buf());
    ctx.paths.create_required_dirs().expect("data dirs");
    agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).expect("db");
    set(&ctx, PLUGINS_ENABLED_SETTING, true);
    let host: Arc<dyn ScriptHost> = Arc::new(QuickJsHost::new());
    let service = PluginService::headless(ctx.clone(), host);
    World { dir, ctx, service }
}

fn set(ctx: &Ctx, key: &str, value: bool) {
    let conn = agora_core::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
    agora_core::db::set_setting(&conn, key, &serde_json::json!(value)).unwrap();
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/provider")
}

fn install_example(world: &World) {
    world
        .service
        .add_development_folder(&example(), true)
        .expect("the shipped example installs");
    assert!(world.service.activate_all().unwrap().is_empty());
}

fn registry(world: &World) -> ProviderRegistry {
    ProviderRegistry::new(&world.ctx, Some(&world.service))
}

fn fabric_instance() -> InstanceManifest {
    serde_json::from_value(serde_json::json!({
        "manifest_version": 2,
        "instance_id": "cozy",
        "name": "Cozy",
        "minecraft_version": "1.21.1",
        "loader": "fabric",
        "loader_version": "0.16.5",
        "mods": []
    }))
    .unwrap()
}

/// A second plugin whose answers are wrong in the ways a buggy or hostile
/// provider's might be.
fn install_misbehaving(world: &World, main: &str) {
    let folder = world.dir.path().join("misbehaving");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("agora-plugin.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "manifest": 1,
            "id": "acme.misbehaving",
            "name": "Misbehaving provider",
            "version": "1.0.0",
            "license": "MIT",
            "apiRange": ">=0.1.1, <0.2",
            "entrypoint": "main.js",
            "capabilities": { "required": ["content:provide", "network"] },
            "network": { "hosts": ["cdn.acme.example"] },
            "contributions": {
                "contentProviders": [{
                    "id": "bad",
                    "title": "Bad Source",
                    "contentTypes": ["mod", "pack"],
                    "exports": { "search": "search", "versions": "versions", "resolve": "resolve" }
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), main).unwrap();
    world
        .service
        .add_development_folder(&folder, true)
        .expect("install");
    assert!(world.service.activate_all().unwrap().is_empty());
}

#[test]
fn a_plugin_provider_sits_beside_the_official_ones_in_the_same_registry() {
    let world = world();
    install_example(&world);
    let descriptors = registry(&world).descriptors();

    let ids: Vec<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
    assert!(ids.contains(&"modrinth"));
    assert!(ids.contains(&"technic"));
    assert!(ids.contains(&PROVIDER));

    let shelf = descriptors.iter().find(|d| d.id == PROVIDER).unwrap();
    assert_eq!(shelf.title, "Example Shelf");
    assert_eq!(
        shelf.origin,
        ProviderOrigin::Plugin {
            plugin_id: "agora.example-provider".into()
        }
    );
    assert_eq!(shelf.download_hosts, vec!["downloads.example.org"]);
    // Official providers are off by default; turning them on is the user's
    // choice, exactly as before providers existed.
    let modrinth = descriptors.iter().find(|d| d.id == "modrinth").unwrap();
    assert!(!modrinth.enabled);
    assert_eq!(modrinth.origin, ProviderOrigin::Official);
}

#[tokio::test]
async fn search_versions_and_detail_flow_through_the_real_script_host() {
    let world = world();
    install_example(&world);
    set(&world.ctx, "network_plugins_enabled", true);
    let registry = registry(&world);
    let shelf = registry.usable(PROVIDER).expect("usable");

    let page = shelf
        .search(SearchRequest {
            content_type: Some("mod".into()),
            limit: 20,
            ..Default::default()
        })
        .await
        .unwrap();
    let titles: Vec<&str> = page.hits.iter().map(|h| h.summary.title.as_str()).collect();
    assert_eq!(titles, vec!["Lantern", "Lib Core"]);

    // A declared filter reaches the plugin; the provider narrows on it.
    let mut filtered = SearchRequest {
        limit: 20,
        ..Default::default()
    };
    filtered
        .filters
        .insert("side".into(), vec!["client".into()]);
    let page = shelf.search(filtered).await.unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].summary.id, "lantern");

    // An undeclared filter, or an undeclared value, is dropped before the
    // plugin ever sees it.
    let mut smuggled = SearchRequest {
        limit: 20,
        ..Default::default()
    };
    smuggled
        .filters
        .insert("side".into(), vec!["server-only".into()]);
    smuggled.filters.insert("other".into(), vec!["x".into()]);
    assert_eq!(shelf.search(smuggled).await.unwrap().hits.len(), 3);

    let versions = shelf
        .versions(VersionsRequest {
            project_id: "lantern".into(),
            minecraft_version: Some("1.21.1".into()),
            loader: Some("fabric".into()),
        })
        .await
        .unwrap();
    assert_eq!(versions.versions.len(), 1);
    assert_eq!(versions.versions[0].dependencies[0].project_id, "lib-core");

    let detail = shelf.project("lantern").await.unwrap();
    assert_eq!(detail.license.as_deref(), Some("MIT"));

    // Declared categories reach Browse's picker; one declared for "all
    // types" is expanded to the provider's own content types by core.
    let categories = agora_core::providers::categories(&registry).await;
    let shelf_categories = categories
        .iter()
        .find(|c| c.provider_id == PROVIDER)
        .expect("the example's categories are offered");
    let cozy = shelf_categories
        .categories
        .iter()
        .find(|c| c.id == "cozy")
        .unwrap();
    assert_eq!(cozy.content_types, vec!["mod", "pack"]);
    let mut by_category = SearchRequest {
        limit: 20,
        category: Some("lighting".into()),
        ..Default::default()
    };
    by_category.sort = agora_core::providers::ProviderSort::Relevance;
    let page = shelf.search(by_category).await.unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].summary.id, "lantern");

    // The provider's declared ranking travels with its descriptor.
    let descriptor = registry.get(PROVIDER).unwrap().descriptor();
    assert_eq!(descriptor.ranking.downloads_ceiling, 100_000);
}

#[tokio::test]
async fn browse_merges_a_plugin_provider_with_no_source_specific_code() {
    let world = world();
    install_example(&world);
    set(&world.ctx, "network_plugins_enabled", true);
    let registry = registry(&world);
    let cache = agora_core::browse_cache::new_cache();

    let result = browse::search(
        &world.ctx,
        &registry,
        &cache,
        browse::BrowseRequest {
            query_key: "all".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    assert!(
        result.provider_failures.is_empty(),
        "{:?}",
        result.provider_failures
    );
    assert_eq!(result.page.items.len(), 3);
    let lantern = result
        .page
        .items
        .iter()
        .find(|i| i.name == "Lantern")
        .unwrap();
    assert_eq!(lantern.id, "provider:agora.example-provider/shelf:lantern");
    assert_eq!(lantern.source, PROVIDER);
    assert_eq!(lantern.provider_title.as_deref(), Some("Example Shelf"));
    assert_eq!(
        lantern.source_page_url.as_deref(),
        Some("https://downloads.example.org/projects/lantern")
    );
}

#[tokio::test]
async fn plugin_network_access_off_takes_the_provider_out_of_browse_with_a_reason() {
    let world = world();
    install_example(&world);
    set(&world.ctx, "network_plugins_enabled", false);
    let registry = registry(&world);
    let shelf = registry
        .descriptors()
        .into_iter()
        .find(|d| d.id == PROVIDER)
        .unwrap();
    assert!(shelf.enabled);
    assert!(shelf.unavailable_reason.is_some());
    assert!(registry.usable(PROVIDER).is_err());
}

#[tokio::test]
async fn a_file_install_resolves_through_the_provider_with_its_dependencies() {
    let world = world();
    install_example(&world);
    set(&world.ctx, "network_plugins_enabled", true);
    let registry = registry(&world);

    let resolution = install::resolve_item(
        &world.ctx,
        &registry,
        &fabric_instance(),
        "provider:agora.example-provider/shelf:lantern",
        None,
    )
    .await
    .expect("resolves");

    let agora_core::install_pipeline::ResolvedArtifact::Download(download) = &resolution.artifact
    else {
        panic!("expected a download");
    };
    assert_eq!(download.filename, "lantern-2.0.0.jar");
    let origin = download.metadata.provider.as_ref().unwrap();
    assert_eq!(origin.provider_id, PROVIDER);
    assert_eq!(origin.project_id, "lantern");
    assert!(
        !origin.low_security,
        "declared host + SHA-512 needs no opt-in"
    );

    // The required library is resolved through the same provider and offered
    // as an install candidate, not left for the user to find.
    assert_eq!(resolution.dependencies.len(), 1);
    assert_eq!(
        resolution.dependencies[0].mod_jar_id,
        "provider:agora.example-provider/shelf:lib-core"
    );
    assert!(matches!(
        resolution.dependencies[0].disposition,
        agora_core::install_pipeline::DepDisposition::InstallCandidate { .. }
    ));
}

#[tokio::test]
async fn a_pack_plan_is_previewed_with_where_its_files_come_from() {
    let world = world();
    install_example(&world);
    set(&world.ctx, "network_plugins_enabled", true);
    let preview = install::preview(
        &world.ctx,
        &registry(&world),
        "provider:agora.example-provider/shelf:cozy-pack",
        None,
        "",
        "",
    )
    .await
    .unwrap();
    assert_eq!(preview.kind, "pack");
    assert_eq!(preview.file_count, 2);
    assert!(preview.warnings.is_empty());
    assert!(preview.low_security.is_empty());
    assert_eq!(preview.hosts.get("downloads.example.org"), Some(&2));
}

#[tokio::test]
async fn reduced_assurance_warns_and_no_integrity_needs_low_security_downloads() {
    let world = world();
    // `x` comes from a host the plugin never declared, with a strong digest.
    // `bare` has no digest at all.
    install_misbehaving(
        &world,
        r#"
        export async function search() {
            return { items: [
                { id: "x", title: "X", contentType: "mod" },
                { id: "bare", title: "Bare", contentType: "mod", lowSecurity: true }
            ] };
        }
        export async function versions() { return { versions: [] }; }
        export async function resolve({ projectId }) {
            const hashes = projectId === "bare" ? {} : { sha512: "a".repeat(128) };
            return {
                kind: "file",
                versionId: "1",
                versionNumber: "1.0",
                contentType: "mod",
                file: { url: "https://somewhere-else.example/x.jar", filename: "x.jar", hashes }
            };
        }
        "#,
    );
    set(&world.ctx, "network_plugins_enabled", true);
    let registry = registry(&world);

    // Reduced assurance is the user's call: it resolves, and the preview
    // carries the warning the review screen shows.
    install::resolve_item(
        &world.ctx,
        &registry,
        &fabric_instance(),
        "provider:acme.misbehaving/bad:x",
        None,
    )
    .await
    .expect("an undeclared host warns rather than blocks");
    let preview = install::preview(
        &world.ctx,
        &registry,
        "provider:acme.misbehaving/bad:x",
        None,
        "1.21.1",
        "fabric",
    )
    .await
    .unwrap();
    assert_eq!(preview.warnings.len(), 1);
    assert!(preview.low_security.is_empty());

    // No integrity information at all needs the explicit opt-in...
    let bare = "provider:acme.misbehaving/bad:bare";
    let refused = install::resolve_item(&world.ctx, &registry, &fabric_instance(), bare, None)
        .await
        .err()
        .expect("a file with no digest needs low security downloads");
    assert!(refused.to_string().contains("low security"), "{refused}");

    // ...and does not even appear in Browse without it.
    let cache = agora_core::browse_cache::new_cache();
    let request = || browse::BrowseRequest {
        query_key: "q".into(),
        ..Default::default()
    };
    let shown = browse::search(&world.ctx, &registry, &cache, request())
        .await
        .unwrap();
    assert!(shown.page.items.iter().all(|i| i.name != "Bare"));

    set(&world.ctx, "allow_unverified_packs", true);
    let shown = browse::search(&world.ctx, &registry, &cache, request())
        .await
        .unwrap();
    assert!(shown.page.items.iter().any(|i| i.name == "Bare"));
    let accepted = install::resolve_item(&world.ctx, &registry, &fabric_instance(), bare, None)
        .await
        .expect("allowed once the user opts into low security downloads");
    let agora_core::install_pipeline::ResolvedArtifact::Download(download) = accepted.artifact
    else {
        panic!("expected a download");
    };
    assert!(download.metadata.provider.unwrap().low_security);
    assert!(download.hashes.values.is_empty());
}

#[tokio::test]
async fn malformed_and_unsafe_answers_are_refused_not_rendered() {
    let world = world();
    install_misbehaving(
        &world,
        r#"
        export async function search() {
            // Not a content type Agora knows.
            return { items: [{ id: "x", title: "X", contentType: "skin" }] };
        }
        export async function versions() { return { versions: "lots" }; }
        export async function resolve() {
            return {
                kind: "pack",
                name: "Escape",
                versionId: "1",
                minecraftVersion: "1.21.1",
                files: [{
                    path: "../../outside.jar",
                    download: {
                        url: "https://cdn.acme.example/o.jar",
                        filename: "o.jar",
                        hashes: { sha512: "a".repeat(128) }
                    }
                }]
            };
        }
        "#,
    );
    set(&world.ctx, "network_plugins_enabled", true);
    let registry = registry(&world);
    let bad = registry.usable("acme.misbehaving/bad").unwrap();

    assert!(bad
        .search(SearchRequest {
            limit: 10,
            ..Default::default()
        })
        .await
        .is_err());
    assert!(bad
        .versions(VersionsRequest {
            project_id: "x".into(),
            minecraft_version: None,
            loader: None,
        })
        .await
        .is_err());
    let escape = bad
        .resolve(ResolveRequest {
            project_id: "x".into(),
            version_id: None,
            minecraft_version: String::new(),
            loader: String::new(),
        })
        .await;
    assert!(
        escape.is_err(),
        "a path outside the pack roots must be refused"
    );

    // Browse keeps working around a provider that answers badly, and says so.
    let cache = agora_core::browse_cache::new_cache();
    let result = browse::search(
        &world.ctx,
        &registry,
        &cache,
        browse::BrowseRequest {
            query_key: "q".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result.provider_failures.len(), 1);
    assert_eq!(
        result.provider_failures[0].provider_id,
        "acme.misbehaving/bad"
    );
}

#[tokio::test]
async fn disabling_the_plugin_removes_its_provider_and_revokes_calls() {
    let world = world();
    install_example(&world);
    set(&world.ctx, "network_plugins_enabled", true);
    let before = registry(&world);
    let shelf = before.usable(PROVIDER).unwrap();

    world
        .service
        .set_enabled(
            &agora_plugin_api::manifest::PluginId::parse("agora.example-provider").unwrap(),
            false,
        )
        .unwrap();

    // Still listed, so it can be switched back on from the provider list,
    // but switched off and unusable.
    let after = registry(&world);
    assert!(!after.get(PROVIDER).unwrap().descriptor().enabled);
    assert!(after.usable(PROVIDER).is_err());
    // A provider handle taken before the switch does not outlive it: the
    // grant is checked on every call.
    assert!(shelf
        .search(SearchRequest {
            limit: 5,
            ..Default::default()
        })
        .await
        .is_err());
}

#[tokio::test]
async fn one_switch_turns_a_plugin_provider_off_and_back_on() {
    let world = world();
    install_example(&world);
    set(&world.ctx, "network_plugins_enabled", true);
    agora_core::providers::set_enabled(
        &world.ctx,
        &registry(&world),
        Some(&world.service),
        PROVIDER,
        false,
    )
    .unwrap();
    assert!(!world.service.list()[0].enabled, "the plugin itself is off");
    agora_core::providers::set_enabled(
        &world.ctx,
        &registry(&world),
        Some(&world.service),
        PROVIDER,
        true,
    )
    .unwrap();
    assert!(registry(&world).usable(PROVIDER).is_ok());

    // An official provider's switch is the setting it always had.
    agora_core::providers::set_enabled(&world.ctx, &registry(&world), None, "technic", true)
        .unwrap();
    assert!(
        registry(&world)
            .get("technic")
            .unwrap()
            .descriptor()
            .enabled
    );
}

#[tokio::test]
async fn plans_from_a_plugin_are_the_same_type_the_official_providers_return() {
    // Not a behavioural test so much as a pin: `InstallPlan` is one enum, and
    // there is no second, privileged way for Agora's own providers to describe
    // an install. If someone adds one, this stops compiling.
    fn accepts(_: &InstallPlan) {}
    let technic_plan =
        agora_core::providers::technic::plan_from_solder(&agora_core::import::TechnicSolderPack {
            display_name: "P".into(),
            minecraft_version: "1.12.2".into(),
            loader: "forge".into(),
            loader_version: "14.23.5.2860".into(),
            mods: vec![],
            slug: "p".into(),
            solder_url: String::new(),
            build: "1".into(),
        })
        .unwrap();
    accepts(&technic_plan);
}
