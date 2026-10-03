//! Browse: the curated catalog and every usable provider, as one ranked,
//! paginated list.
//!
//! This used to live in the desktop adapter as a Modrinth branch and a
//! Technic branch. It belongs in core — the CLI and MCP deserve the same
//! list — and it no longer names a source: each provider is asked the same
//! question, and its answer is paged by its own cursor.

use crate::browse_cache::{
    self, BrowseFilters, BrowseItem, BrowsePage, ProviderCursor, SharedBrowseCache, PAGE_SIZE,
};
use agora_core::ctx::Ctx;
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::providers::{ProviderDescriptor, ProviderRegistry, ProviderSort, SearchRequest};
use agora_core::registry::{RegistryService, SortOption};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Curated items fetched per query. The catalog is small and fetched in full.
const CURATED_LIMIT: i64 = 100;

/// What the user asked Browse for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseRequest {
    pub query_key: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub mc_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    /// Values for providers' declared filters, by provider id then filter id.
    #[serde(default)]
    pub provider_filters: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// A provider that could not answer. Browse still shows everyone else.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFailure {
    pub provider_id: String,
    pub title: String,
    pub message: String,
}

/// One page of Browse, plus which providers failed to contribute to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseResult {
    #[serde(flatten)]
    pub page: BrowsePage,
    #[serde(default)]
    pub provider_failures: Vec<ProviderFailure>,
}

/// Curated download strategies the user has left enabled (Axis A, §20.2).
/// A missing setting defaults to on, so curated content never silently
/// disappears.
pub fn enabled_curated_strategies(ctx: &Ctx) -> Vec<String> {
    let settings = agora_core::settings::SettingsService::new(ctx.clone());
    agora_core::registry::CURATED_DOWNLOAD_STRATEGIES
        .iter()
        .map(|strategy| strategy.to_string())
        .filter(|strategy| {
            settings
                .get_bool_or(&format!("curated_source_{strategy}_enabled"), true)
                .unwrap_or(true)
        })
        .collect()
}

/// Browse's sort keys to the curated catalog's.
pub fn curated_sort(sort: &str) -> SortOption {
    match sort {
        "velocity" => SortOption::Velocity,
        "most_downvoted" => SortOption::MostDownvoted,
        "newest" => SortOption::Newest,
        "most_upvoted" => SortOption::MostUpvoted,
        _ => SortOption::NetScore,
    }
}

/// Browse's sort keys to the provider vocabulary.
///
/// Chunks are sorted only within themselves, so the closer a provider's
/// upstream order is to ours, the smaller the inversions across a chunk
/// boundary. Measured against Modrinth, `follows` returns sodium -> fabric-api
/// -> iris -> modmenu, which tracks the blended score far better than
/// `downloads` (which leads with fabric-api, a library the ranker demotes).
pub fn provider_sort(sort: &str) -> ProviderSort {
    match sort {
        "downloads" => ProviderSort::Downloads,
        "newest" => ProviderSort::Newest,
        "updated" | "velocity" => ProviderSort::Updated,
        // Every blended sort wants engagement-led ordering.
        "net_score" | "most_upvoted" | "most_downvoted" | "follows" => ProviderSort::Follows,
        _ => ProviderSort::Relevance,
    }
}

/// The closest ordering a provider actually supports.
fn supported_sort(descriptor: &ProviderDescriptor, wanted: ProviderSort) -> ProviderSort {
    if descriptor.sorts.contains(&wanted) {
        return wanted;
    }
    // Popularity sorts are interchangeable enough to substitute; anything
    // else degrades to relevance, which every provider can do.
    let fallback = match wanted {
        ProviderSort::Follows => Some(ProviderSort::Downloads),
        ProviderSort::Downloads => Some(ProviderSort::Follows),
        ProviderSort::Newest => Some(ProviderSort::Updated),
        ProviderSort::Updated => Some(ProviderSort::Newest),
        ProviderSort::Relevance => None,
    };
    fallback
        .filter(|sort| descriptor.sorts.contains(sort))
        .or_else(|| {
            descriptor
                .sorts
                .contains(&ProviderSort::Relevance)
                .then_some(ProviderSort::Relevance)
        })
        .unwrap_or_else(|| descriptor.sorts.first().copied().unwrap_or_default())
}

/// Whether a provider has anything of the requested content type.
fn offers(descriptor: &ProviderDescriptor, content_type: Option<&str>) -> bool {
    content_type.is_none_or(|wanted| descriptor.content_types.iter().any(|t| t == wanted))
}

fn search_request(
    filters: &BrowseFilters,
    descriptor: &ProviderDescriptor,
    offset: u32,
) -> SearchRequest {
    SearchRequest {
        query: filters.query.clone(),
        content_type: filters.content_type.clone(),
        minecraft_version: filters.mc_version.clone(),
        loader: filters.loader.clone(),
        category: filters.category.clone(),
        sort: supported_sort(descriptor, provider_sort(&filters.sort)),
        offset,
        limit: PAGE_SIZE as u32,
        filters: filters
            .provider_filters
            .get(&descriptor.id)
            .cloned()
            .unwrap_or_default(),
    }
}

/// Ask each provider for one page, concurrently. A failure is reported, not
/// fatal: one source being down must never empty Browse.
async fn fetch_round(
    registry: &ProviderRegistry,
    filters: &BrowseFilters,
    asks: Vec<(String, u32)>,
    show_low_security: bool,
) -> (
    Vec<BrowseItem>,
    BTreeMap<String, ProviderCursor>,
    Vec<ProviderFailure>,
) {
    let mut tasks = tokio::task::JoinSet::new();
    for (provider_id, offset) in asks {
        let Ok(provider) = registry.usable(&provider_id) else {
            continue;
        };
        let descriptor = provider.descriptor();
        let request = search_request(filters, &descriptor, offset);
        tasks.spawn(async move {
            let result = provider.search(request).await;
            (descriptor, offset, result)
        });
    }

    let mut items = Vec::new();
    let mut cursors = BTreeMap::new();
    let mut failures = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        let Ok((descriptor, offset, result)) = joined else {
            continue;
        };
        match result {
            Ok(page) => {
                let returned = page.hits.len() as u32;
                // A provider that says "more" but returned nothing would loop
                // forever; an empty page is the end whatever it claims.
                let has_more = descriptor.paginates && page.has_more && returned > 0;
                cursors.insert(
                    descriptor.id.clone(),
                    ProviderCursor {
                        offset: offset + returned,
                        has_more,
                    },
                );
                items.extend(
                    page.hits
                        .into_iter()
                        // Content with no integrity information appears only
                        // for users who allowed low security downloads.
                        .filter(|hit| show_low_security || !hit.summary.low_security)
                        .map(|hit| browse_cache::item_from_hit(&descriptor, hit)),
                );
            }
            Err(error) => {
                cursors.insert(
                    descriptor.id.clone(),
                    ProviderCursor {
                        offset,
                        has_more: false,
                    },
                );
                failures.push(ProviderFailure {
                    provider_id: descriptor.id.clone(),
                    title: descriptor.title.clone(),
                    message: error.to_string(),
                });
            }
        }
    }
    (items, cursors, failures)
}

fn stale() -> LauncherError {
    LauncherError::Generic {
        code: "ERR_BROWSE_STALE".into(),
        message: "Browse query changed before pagination completed.".into(),
    }
}

/// Run a new Browse query and return its first page.
pub async fn search(
    ctx: &Ctx,
    registry: &ProviderRegistry,
    cache: &SharedBrowseCache,
    request: BrowseRequest,
) -> LauncherResult<BrowseResult> {
    let filters = BrowseFilters {
        query: request.query.clone().unwrap_or_default(),
        content_type: request.content_type.clone(),
        category: request.category.clone(),
        sort: request.sort.clone().unwrap_or_else(|| "net_score".into()),
        mc_version: request.mc_version.clone(),
        loader: request.loader.clone(),
        provider_filters: request.provider_filters.clone(),
    };
    let providers: Vec<String> = registry
        .usable_providers()
        .into_iter()
        .filter(|(descriptor, _)| offers(descriptor, filters.content_type.as_deref()))
        .map(|(descriptor, _)| descriptor.id)
        .collect();

    let catalog = RegistryService::new(ctx.clone());
    let mean_approval = catalog.mean_approval();
    let curated = match catalog.browse_items(
        filters.content_type.as_deref(),
        filters.category.as_deref(),
        &curated_sort(&filters.sort),
        &enabled_curated_strategies(ctx),
        filters.mc_version.as_deref(),
        filters.loader.as_deref(),
        request.query.as_deref(),
        CURATED_LIMIT,
    ) {
        Ok(items) => items,
        // No catalog on disk yet. With a provider enabled, Browse degrades to
        // that provider rather than failing: an absent catalog is a missing
        // ingredient here, not a broken query.
        Err(LauncherError::RegistryMissing) if !providers.is_empty() => Vec::new(),
        Err(e) => {
            return Err(LauncherError::Generic {
                code: "ERR_REGISTRY".into(),
                message: e.to_string(),
            })
        }
    };

    let (provider_items, cursors, provider_failures) = fetch_round(
        registry,
        &filters,
        providers.into_iter().map(|id| (id, 0)).collect(),
        agora_core::providers::low_security_allowed(ctx),
    )
    .await;

    let merged = browse_cache::merge_items(curated, provider_items, mean_approval);
    browse_cache::load_initial(cache, request.query_key.clone(), merged, filters, cursors).await;

    let mut page = browse_cache::get_page(cache, 0).await;
    page.has_more = page.has_more || cache.read().await.has_more_upstream();
    Ok(BrowseResult {
        page,
        provider_failures,
    })
}

/// Serve page `page_index` of the current query, fetching from providers
/// until it is full or every provider is exhausted.
pub async fn load_more(
    ctx: &Ctx,
    registry: &ProviderRegistry,
    cache: &SharedBrowseCache,
    query_key: &str,
    page_index: usize,
) -> LauncherResult<BrowseResult> {
    let required_end = (page_index + 1) * PAGE_SIZE;
    let mean_approval = RegistryService::new(ctx.clone()).mean_approval();
    let mut provider_failures = Vec::new();

    loop {
        let (filters, asks) = {
            let c = cache.read().await;
            if c.query_key != query_key {
                return Err(stale());
            }
            // The carry-forward buffer may already cover the requested page:
            // curated is fetched in full and non-paginating providers arrive
            // in one shot, so several pages can need no network at all.
            if c.items.len() + c.buffer.len() >= required_end {
                break;
            }
            let asks: Vec<(String, u32)> = c
                .cursors
                .iter()
                .filter(|(_, cursor)| cursor.has_more)
                .map(|(id, cursor)| (id.clone(), cursor.offset))
                .collect();
            (c.filters.clone(), asks)
        };
        if asks.is_empty() {
            break;
        }
        let asked: Vec<String> = asks.iter().map(|(id, _)| id.clone()).collect();
        let (items, mut cursors, failures) = fetch_round(
            registry,
            &filters,
            asks,
            agora_core::providers::low_security_allowed(ctx),
        )
        .await;
        // A provider that vanished since the query started (disabled, plugin
        // removed) is simply finished; it must not keep the loop alive.
        for id in asked {
            cursors.entry(id).or_insert(ProviderCursor {
                offset: 0,
                has_more: false,
            });
        }
        provider_failures.extend(failures);
        if !browse_cache::append_items(cache, query_key, items, cursors, mean_approval).await {
            return Err(stale());
        }
    }

    // Promote buffered items into the displayed list before slicing the page.
    if !browse_cache::drain_buffer(cache, query_key, required_end).await {
        return Err(stale());
    }
    let mut page = browse_cache::get_page(cache, page_index).await;
    let c = cache.read().await;
    if c.query_key != query_key {
        return Err(stale());
    }
    // `get_page` already accounts for the buffer; ORing the upstream flag on
    // top only adds the "more to fetch" case.
    page.has_more = page.has_more || c.has_more_upstream();
    Ok(BrowseResult {
        page,
        provider_failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::providers::{
        ContentProvider, InstallPlan, ProjectDetail, ProviderHit, ProviderOrigin, ProviderPage,
        ResolveRequest, VersionsRequest, VersionsResponse,
    };
    use agora_plugin_api::provider::ProjectSummary;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A provider serving `total` numbered projects, `PAGE_SIZE` at a time.
    struct Numbered {
        id: &'static str,
        total: u32,
        paginates: bool,
        fail: bool,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ContentProvider for Numbered {
        fn descriptor(&self) -> ProviderDescriptor {
            ProviderDescriptor {
                id: self.id.into(),
                title: self.id.into(),
                description: None,
                origin: ProviderOrigin::Plugin {
                    plugin_id: "test.plugin".into(),
                },
                content_types: vec!["mod".into()],
                filters: vec![],
                sorts: vec![ProviderSort::Relevance],
                paginates: self.paginates,
                ranking: Default::default(),
                download_hosts: vec![],
                enabled: true,
                unavailable_reason: None,
            }
        }
        async fn search(&self, request: SearchRequest) -> LauncherResult<ProviderPage> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(LauncherError::Generic {
                    code: "ERR_TEST".into(),
                    message: "down".into(),
                });
            }
            let end = if self.paginates {
                (request.offset + request.limit).min(self.total)
            } else {
                self.total
            };
            let hits = (request.offset..end)
                .map(|n| ProviderHit {
                    summary: ProjectSummary {
                        id: format!("{n}"),
                        title: format!("{} {n}", self.id),
                        content_type: "mod".into(),
                        downloads: Some(u64::from(1_000 - n)),
                        ..Default::default()
                    },
                    native: None,
                })
                .collect();
            Ok(ProviderPage {
                hits,
                total: Some(u64::from(self.total)),
                has_more: end < self.total,
            })
        }
        async fn project(&self, _: &str) -> LauncherResult<ProjectDetail> {
            unimplemented!()
        }
        async fn versions(&self, _: VersionsRequest) -> LauncherResult<VersionsResponse> {
            unimplemented!()
        }
        async fn resolve(&self, _: ResolveRequest) -> LauncherResult<InstallPlan> {
            unimplemented!()
        }
    }

    fn numbered(id: &'static str, total: u32, paginates: bool, fail: bool) -> Arc<Numbered> {
        Arc::new(Numbered {
            id,
            total,
            paginates,
            fail,
            calls: AtomicUsize::new(0),
        })
    }

    fn ctx() -> (Ctx, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("agora-browse-tests-{}", uuid::Uuid::new_v4()));
        let ctx = Ctx::for_testing(root.clone());
        agora_core::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        (ctx, root)
    }

    #[tokio::test]
    async fn every_item_from_every_provider_is_reachable_and_failures_are_reported() {
        let (ctx, root) = ctx();
        let paging = numbered("a.one/paging", 45, true, false);
        let oneshot = numbered("b.two/oneshot", 7, false, false);
        let broken = numbered("c.three/broken", 10, true, true);
        let registry =
            ProviderRegistry::from_providers(vec![paging.clone(), oneshot.clone(), broken.clone()]);
        let cache = browse_cache::new_cache();

        let first = search(
            &ctx,
            &registry,
            &cache,
            BrowseRequest {
                query_key: "q".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(first.page.items.len(), PAGE_SIZE);
        assert!(first.page.has_more);
        assert_eq!(first.provider_failures.len(), 1);
        assert_eq!(first.provider_failures[0].provider_id, "c.three/broken");

        let mut seen: Vec<String> = first.page.items.iter().map(|i| i.id.clone()).collect();
        let mut page_index = 1;
        loop {
            let next = load_more(&ctx, &registry, &cache, "q", page_index)
                .await
                .unwrap();
            seen.extend(next.page.items.iter().map(|i| i.id.clone()));
            if !next.page.has_more {
                break;
            }
            page_index += 1;
            assert!(page_index < 20, "paging never finished");
        }
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            45 + 7,
            "every item must be reachable exactly once"
        );
        // A provider that cannot paginate is asked once per query, however
        // many pages are served.
        assert_eq!(oneshot.calls.load(Ordering::SeqCst), 1);
        // A failed provider is not retried on every page.
        assert_eq!(broken.calls.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn a_stale_query_is_refused() {
        let (ctx, root) = ctx();
        let registry = ProviderRegistry::from_providers(vec![numbered("a.b/c", 50, true, false)]);
        let cache = browse_cache::new_cache();
        search(
            &ctx,
            &registry,
            &cache,
            BrowseRequest {
                query_key: "new".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(load_more(&ctx, &registry, &cache, "old", 1).await.is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn sorts_degrade_to_something_the_provider_supports() {
        let mut descriptor = numbered("a.b/c", 0, true, false).descriptor();
        descriptor.sorts = vec![ProviderSort::Relevance, ProviderSort::Downloads];
        assert_eq!(
            supported_sort(&descriptor, ProviderSort::Follows),
            ProviderSort::Downloads
        );
        assert_eq!(
            supported_sort(&descriptor, ProviderSort::Newest),
            ProviderSort::Relevance
        );
    }
}
