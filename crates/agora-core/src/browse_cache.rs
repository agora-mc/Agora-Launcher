use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::modrinth::ModrinthSearchResult;
use crate::ranking::{self, RankingInput, RankingProfile, ScoreBreakdown};
use crate::registry::RegistryItem;
use crate::technic::TechnicSearchResult;

pub const PAGE_SIZE: usize = 20;

#[derive(Debug, Clone)]
struct NormalizedPresentation {
    hero_image_url: Option<String>,
    author: Option<String>,
    categories: Vec<String>,
    downloads: Option<i64>,
    follows: Option<i64>,
    upvotes: Option<i64>,
    downvotes: Option<i64>,
    net_score: Option<i64>,
    supported_versions: Vec<String>,
    source_page_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseItem {
    pub id: String,
    /// `curated`, or the id of the provider the item came from (`modrinth`,
    /// `technic`, `<plugin>/<provider>`).
    pub source: String,
    /// The provider this item came from, or `None` for a curated item. A
    /// curated item that Modrinth also lists keeps `None`: curation wins
    /// identity ties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// The provider's display name, so a card can say where it came from
    /// without a second lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_title: Option<String>,
    pub registry_item: Option<RegistryItem>,
    pub modrinth_result: Option<ModrinthSearchResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technic_result: Option<TechnicSearchResult>,
    pub name: String,
    pub icon_url: Option<String>,
    pub description: Option<String>,
    pub content_type: String,
    pub hero_image_url: Option<String>,
    pub author: Option<String>,
    pub categories: Vec<String>,
    pub downloads: Option<i64>,
    pub follows: Option<i64>,
    pub upvotes: Option<i64>,
    pub downvotes: Option<i64>,
    pub net_score: Option<i64>,
    pub supported_versions: Vec<String>,
    pub source_page_url: Option<String>,
    /// Unified 0-100 cross-source score; the sort key for the merged list.
    #[serde(default)]
    pub score: f64,
    /// Why the item scored what it did. Present so a surprising ordering can be
    /// explained without re-deriving the math by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_breakdown: Option<ScoreBreakdown>,
    /// The popularity scale of the provider this came from. `None` uses the
    /// default profile, which is also what curated items are ranked on.
    #[serde(skip)]
    pub ranking_profile: Option<RankingProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePage {
    pub items: Vec<BrowseItem>,
    pub total: usize,
    pub page: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowseFilters {
    pub query: String,
    pub content_type: Option<String>,
    pub category: Option<String>,
    pub sort: String,
    pub mc_version: Option<String>,
    pub loader: Option<String>,
    /// Values for providers' own declared filters, by provider id then
    /// filter id.
    #[serde(default)]
    pub provider_filters: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// Where one provider's paging stands for the current query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCursor {
    pub offset: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrowseCache {
    /// Immutable identity of the filters that produced this cache.
    pub query_key: String,
    pub items: Vec<BrowseItem>,
    pub total: usize,
    pub filters: BrowseFilters,
    /// Paging state per provider, by provider id. A provider that cannot
    /// paginate is fetched once per query and recorded with `has_more: false`,
    /// then drained through `buffer` — the general form of what used to be a
    /// Technic special case.
    pub cursors: BTreeMap<String, ProviderCursor>,
    /// Everything fetched but not yet shown, scored and waiting for the next
    /// chunk. Holds curated and uncurated alike — a curated item that scores
    /// below a chunk's floor waits here and surfaces once the incoming pages
    /// drop to its level, instead of all curated content dumping at the top.
    pub buffer: Vec<BrowseItem>,
}

impl BrowseCache {
    /// Whether any provider still has results upstream.
    pub fn has_more_upstream(&self) -> bool {
        self.cursors.values().any(|cursor| cursor.has_more)
    }
}

pub type SharedBrowseCache = Arc<RwLock<BrowseCache>>;

pub fn new_cache() -> SharedBrowseCache {
    Arc::new(RwLock::new(BrowseCache::default()))
}

pub fn normalize_modrinth_content_type(project_type: &str) -> &str {
    match project_type {
        "modpack" => "pack",
        "minecraft_java_server" => "server",
        other => other,
    }
}

fn normalized_https_url(value: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(value).ok()?;
    (parsed.scheme() == "https").then(|| parsed.to_string())
}

fn registry_gallery_urls(item: &RegistryItem) -> Vec<String> {
    item.gallery_urls_json
        .as_deref()
        .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|url| normalized_https_url(&url))
        .collect()
}

fn registry_supported_versions(item: &RegistryItem) -> Vec<String> {
    let Some(json) = item.compatible_versions_json.as_deref() else {
        return Vec::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    let mut versions = Vec::new();
    for version in entries
        .iter()
        .filter_map(|entry| entry.get("mc_version").and_then(|value| value.as_str()))
    {
        if !versions.iter().any(|existing| existing == version) {
            versions.push(version.to_string());
        }
    }
    versions
}

fn modrinth_page_url(item: &ModrinthSearchResult) -> Option<String> {
    if item.slug.is_empty() {
        None
    } else {
        Some(format!(
            "https://modrinth.com/{}/{}",
            item.project_type, item.slug
        ))
    }
}

fn normalized_presentation(
    registry_item: Option<&RegistryItem>,
    modrinth_item: Option<&ModrinthSearchResult>,
) -> NormalizedPresentation {
    let registry_gallery = registry_item.map(registry_gallery_urls).unwrap_or_default();
    let hero_image_url = modrinth_item
        .and_then(|item| item.featured_gallery.as_deref())
        .and_then(normalized_https_url)
        .or_else(|| registry_gallery.first().cloned())
        .or_else(|| {
            modrinth_item.and_then(|item| {
                item.gallery
                    .iter()
                    .find_map(|url| normalized_https_url(url))
            })
        });
    let supported_versions = registry_item
        .map(registry_supported_versions)
        .filter(|versions| !versions.is_empty())
        .or_else(|| modrinth_item.map(|item| item.versions.clone()))
        .unwrap_or_default();
    let source_page_url = registry_item
        .and_then(|item| item.page_url.as_deref())
        .and_then(normalized_https_url)
        .or_else(|| modrinth_item.and_then(modrinth_page_url));

    NormalizedPresentation {
        hero_image_url,
        author: modrinth_item
            .map(|item| item.author.clone())
            .filter(|author| !author.is_empty()),
        categories: modrinth_item
            .map(|item| item.categories.clone())
            .unwrap_or_default(),
        downloads: modrinth_item.map(|item| item.downloads),
        follows: modrinth_item.map(|item| item.follows),
        upvotes: registry_item.map(|item| item.upvotes),
        downvotes: registry_item.map(|item| item.downvotes),
        net_score: registry_item.map(|item| item.net_score),
        supported_versions,
        source_page_url,
    }
}

pub fn item_from_modrinth(item: ModrinthSearchResult) -> BrowseItem {
    let pres = normalized_presentation(None, Some(&item));
    BrowseItem {
        id: item.project_id.clone(),
        source: "modrinth".to_string(),
        provider_id: Some("modrinth".to_string()),
        provider_title: Some("Modrinth".to_string()),
        registry_item: None,
        modrinth_result: Some(item.clone()),
        name: item.title.clone(),
        icon_url: item.icon_url.clone(),
        description: Some(item.description.clone()),
        content_type: normalize_modrinth_content_type(&item.project_type).to_string(),
        hero_image_url: pres.hero_image_url,
        author: pres.author,
        categories: pres.categories,
        downloads: pres.downloads,
        follows: pres.follows,
        upvotes: pres.upvotes,
        downvotes: pres.downvotes,
        net_score: pres.net_score,
        supported_versions: pres.supported_versions,
        source_page_url: pres.source_page_url,
        technic_result: None,
        score: 0.0,
        score_breakdown: None,
        ranking_profile: None,
    }
}

/// Build a `BrowseItem` for a Technic pack.
///
/// Technic packs have no project id, so the slug is namespaced to keep it from
/// colliding with a Modrinth project id or a registry item id.
pub fn item_from_technic(item: TechnicSearchResult) -> BrowseItem {
    BrowseItem {
        id: format!("technic:{}", item.slug),
        source: "technic".to_string(),
        provider_id: Some("technic".to_string()),
        provider_title: Some("Technic".to_string()),
        registry_item: None,
        modrinth_result: None,
        name: item.title.clone(),
        icon_url: item.icon_url.clone(),
        description: Some(item.description.clone()),
        // Technic only distributes modpacks.
        content_type: "pack".to_string(),
        hero_image_url: None,
        author: item.author.clone(),
        categories: item.tags.clone(),
        downloads: Some(item.installs as i64),
        follows: Some(item.likes as i64),
        upvotes: None,
        downvotes: None,
        net_score: None,
        supported_versions: Vec::new(),
        source_page_url: Some(item.page_url.clone()),
        technic_result: Some(item),
        score: 0.0,
        score_breakdown: None,
        ranking_profile: None,
    }
}

/// Build a `BrowseItem` for any provider's search hit.
///
/// The official providers carry their original payloads, which older screens
/// still read, so they keep their established ids and shapes. Every other
/// provider's item is built from the provider vocabulary alone and gets a
/// `provider:<provider>:<project>` id, which the detail page routes on.
pub fn item_from_hit(
    descriptor: &crate::providers::ProviderDescriptor,
    hit: crate::providers::ProviderHit,
) -> BrowseItem {
    let provider_id = descriptor.id.as_str();
    let provider_title = descriptor.title.as_str();
    let mut item = match hit.native {
        Some(crate::providers::NativeHit::Modrinth(result)) => item_from_modrinth(result),
        Some(crate::providers::NativeHit::Technic(result)) => item_from_technic(result),
        None => {
            let summary = hit.summary;
            BrowseItem {
                id: crate::providers::item_id(provider_id, &summary.id),
                source: provider_id.to_string(),
                provider_id: Some(provider_id.to_string()),
                provider_title: Some(provider_title.to_string()),
                registry_item: None,
                modrinth_result: None,
                technic_result: None,
                name: summary.title,
                // Only HTTPS images are ever put in front of the WebView.
                icon_url: summary.icon_url.and_then(|u| normalized_https_url(&u)),
                description: summary.description,
                content_type: summary.content_type,
                hero_image_url: summary
                    .hero_image_url
                    .and_then(|u| normalized_https_url(&u)),
                author: summary.author,
                categories: summary.categories,
                downloads: summary.downloads.and_then(|d| i64::try_from(d).ok()),
                follows: summary.follows.and_then(|f| i64::try_from(f).ok()),
                upvotes: None,
                downvotes: None,
                net_score: None,
                supported_versions: summary.minecraft_versions,
                source_page_url: summary.page_url.and_then(|u| normalized_https_url(&u)),
                score: 0.0,
                score_breakdown: None,
                ranking_profile: None,
            }
        }
    };
    // Every provider's items are ranked on the scale that provider declared.
    item.ranking_profile = Some(descriptor.ranking.clone());
    item
}

/// Collect the ranking signals for an already-assembled `BrowseItem`.
///
/// Curated items take their popularity from the linked Modrinth project when
/// one exists. That matters: every registry item currently has zero votes, so
/// without this fallback all curated content would tie at the band floor.
fn ranking_input(item: &BrowseItem) -> RankingInput {
    let curated = item.source == "curated";
    // Category tags come from the Modrinth payload. A curated item matched to a
    // Modrinth project carries them; one without a match has none, so the
    // library penalty simply does not apply. `RegistryItem` has no category
    // list of its own (curated taxonomy lives in the `item_categories` table).
    let categories = item.categories.clone();
    RankingInput {
        downloads: item.downloads,
        endorsements: item.follows,
        profile: item.ranking_profile.clone().unwrap_or_default(),
        categories,
        curated,
        upvotes: item.upvotes.unwrap_or(0),
        downvotes: item.downvotes.unwrap_or(0),
    }
}

/// Score every item in place and sort highest-first.
pub fn score_and_sort(items: &mut [BrowseItem], registry_mean_approval: f64) {
    for item in items.iter_mut() {
        let breakdown = ranking::score_item(&ranking_input(item), registry_mean_approval);
        item.score = breakdown.score;
        item.score_breakdown = Some(breakdown);
    }
    ranking::sort_by_score(items, |item| item.score, |item| item.name.as_str());
}

/// Merge the curated catalog with every provider's hits into one ranked list.
///
/// A curated item whose registry row names a Modrinth project, and which that
/// project's hit also appears for, is emitted once, tagged `curated`, and
/// ranked by the curated band — curation wins identity ties. That match is
/// driven by the *registry's* `modrinth_id` column, which is catalog data, not
/// by anything about the provider. The result is sorted by the unified score.
pub fn merge_items(
    registry_items: Vec<RegistryItem>,
    provider_items: Vec<BrowseItem>,
    registry_mean_approval: f64,
) -> Vec<BrowseItem> {
    let mut registry_by_modrinth_id = HashMap::new();
    for ri in &registry_items {
        if let Some(ref mid) = ri.modrinth_id {
            registry_by_modrinth_id.insert(mid.clone(), ri.clone());
        }
    }

    let mut matched_ids = std::collections::HashSet::new();
    let mut merged = Vec::new();

    for provider_item in provider_items {
        let Some(mr) = provider_item.modrinth_result.clone() else {
            merged.push(provider_item);
            continue;
        };
        if let Some(matched) = registry_by_modrinth_id.get(&mr.project_id) {
            matched_ids.insert(matched.id.clone());
            let pres = normalized_presentation(Some(matched), Some(&mr));
            merged.push(BrowseItem {
                id: matched.id.clone(),
                source: "curated".to_string(),
                provider_id: None,
                provider_title: None,
                registry_item: Some(matched.clone()),
                modrinth_result: Some(mr.clone()),
                name: matched.name.clone(),
                icon_url: matched.icon_url.clone().or(mr.icon_url.clone()),
                description: matched
                    .description
                    .clone()
                    .or_else(|| Some(mr.description.clone())),
                content_type: matched.content_type.clone(),
                hero_image_url: pres.hero_image_url,
                author: pres.author,
                categories: pres.categories,
                downloads: pres.downloads,
                follows: pres.follows,
                upvotes: pres.upvotes,
                downvotes: pres.downvotes,
                net_score: pres.net_score,
                supported_versions: pres.supported_versions,
                source_page_url: pres.source_page_url,
                technic_result: None,
                score: 0.0,
                score_breakdown: None,
                ranking_profile: None,
            });
        } else {
            merged.push(provider_item);
        }
    }

    for ri in registry_items {
        if !matched_ids.contains(&ri.id) {
            let pres = normalized_presentation(Some(&ri), None);
            merged.push(BrowseItem {
                id: ri.id.clone(),
                source: "curated".to_string(),
                provider_id: None,
                provider_title: None,
                registry_item: Some(ri.clone()),
                modrinth_result: None,
                name: ri.name.clone(),
                icon_url: ri.icon_url.clone(),
                description: ri.description.clone(),
                content_type: ri.content_type.clone(),
                hero_image_url: pres.hero_image_url,
                author: pres.author,
                categories: pres.categories,
                downloads: pres.downloads,
                follows: pres.follows,
                upvotes: pres.upvotes,
                downvotes: pres.downvotes,
                net_score: pres.net_score,
                supported_versions: pres.supported_versions,
                source_page_url: pres.source_page_url,
                technic_result: None,
                score: 0.0,
                score_breakdown: None,
                ranking_profile: None,
            });
        }
    }

    score_and_sort(&mut merged, registry_mean_approval);
    merged
}

/// Split a freshly-scored pile into "show now" and "hold for the next chunk".
///
/// Sorting each chunk only within itself means a later chunk can contain an
/// item that outranks something already displayed. That is inherent to
/// append-only paging over sources we do not control, and it is the price of
/// never moving an item under the user's cursor mid-scroll. Requesting the
/// upstream sort closest to ours keeps the inversions small. Do NOT "fix" this
/// by re-sorting the whole list — that reintroduces the jumping.
pub fn split_chunk(mut pile: Vec<BrowseItem>, take: usize) -> (Vec<BrowseItem>, Vec<BrowseItem>) {
    if pile.len() <= take {
        return (pile, Vec::new());
    }
    let held = pile.split_off(take);
    (pile, held)
}

/// Load the first page of browse results into the cache.
///
/// `items` is the already-merged, already-scored first pile (see
/// [`merge_items`]); `cursors` records where each provider's paging stands.
pub async fn load_initial(
    cache: &SharedBrowseCache,
    query_key: String,
    items: Vec<BrowseItem>,
    filters: BrowseFilters,
    cursors: BTreeMap<String, ProviderCursor>,
) {
    // Curated is fetched in full up front, so the first pile is usually far
    // larger than one page. Show the best PAGE_SIZE and hold the rest.
    let (shown, held) = split_chunk(items, PAGE_SIZE);
    let mut c = cache.write().await;
    c.query_key = query_key;
    c.total = shown.len() + held.len();
    c.items = shown;
    c.buffer = held;
    c.filters = filters;
    c.cursors = cursors;
}

/// Append more provider items only when the cache still belongs to the
/// expected query. Returns false when a newer query replaced the cache.
///
/// `cursors` replaces the paging state of the providers that were just asked;
/// providers not named keep theirs.
pub async fn append_items(
    cache: &SharedBrowseCache,
    expected_query_key: &str,
    new_items: Vec<BrowseItem>,
    cursors: BTreeMap<String, ProviderCursor>,
    registry_mean_approval: f64,
) -> bool {
    let mut c = cache.write().await;
    if c.query_key != expected_query_key {
        return false;
    }
    let mut existing_ids: std::collections::HashSet<(String, String)> = c
        .items
        .iter()
        .chain(c.buffer.iter())
        .map(|i| (i.source.clone(), i.id.clone()))
        .collect();

    // The next chunk is the carry-forward buffer plus whatever just arrived,
    // scored and sorted together. Items already on screen are never touched.
    let mut pile = std::mem::take(&mut c.buffer);
    for item in new_items {
        if existing_ids.insert((item.source.clone(), item.id.clone())) {
            pile.push(item);
        }
    }
    score_and_sort(&mut pile, registry_mean_approval);

    c.cursors.extend(cursors);
    // Drain rule: once upstream is exhausted the buffer must be flushed, or
    // items held back earlier would never appear at all.
    let take = if c.has_more_upstream() {
        PAGE_SIZE
    } else {
        pile.len()
    };
    let (shown, held) = split_chunk(pile, take);
    c.items.extend(shown);
    c.buffer = held;
    c.total = c.items.len() + c.buffer.len();
    true
}

/// Move buffered items into the displayed list until it covers `required_end`.
///
/// The buffer is already scored and sorted, so taking from its front preserves
/// ranking without re-sorting anything already on screen. Needed because
/// curated is fetched in full and Technic arrives in one shot, so several pages
/// can be served with no further network fetch — in which case `append_items`
/// never runs and nothing else would promote them.
pub async fn drain_buffer(
    cache: &SharedBrowseCache,
    expected_query_key: &str,
    required_end: usize,
) -> bool {
    let mut c = cache.write().await;
    if c.query_key != expected_query_key {
        return false;
    }
    while c.items.len() < required_end && !c.buffer.is_empty() {
        let take = std::cmp::min(PAGE_SIZE, c.buffer.len());
        let promoted: Vec<BrowseItem> = c.buffer.drain(..take).collect();
        c.items.extend(promoted);
    }
    c.total = c.items.len() + c.buffer.len();
    true
}

/// Get a page of results from the cache.
pub async fn get_page(cache: &SharedBrowseCache, page: usize) -> BrowsePage {
    let c = cache.read().await;
    let start = page * PAGE_SIZE;
    let end = std::cmp::min(start + PAGE_SIZE, c.items.len());
    let items = if start < c.items.len() {
        c.items[start..end].to_vec()
    } else {
        Vec::new()
    };
    BrowsePage {
        items,
        total: c.total,
        page,
        has_more: end < c.items.len() || !c.buffer.is_empty(),
    }
}

/// Get all cached items (for CLI/MCP use).
pub async fn get_all(cache: &SharedBrowseCache) -> Vec<BrowseItem> {
    let c = cache.read().await;
    c.items.clone()
}

/// Invalidate the cache (e.g., on filter change).
pub async fn invalidate(cache: &SharedBrowseCache) {
    let mut c = cache.write().await;
    *c = BrowseCache::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: usize) -> BrowseItem {
        BrowseItem {
            id: format!("item-{id}"),
            source: "curated".into(),
            provider_id: None,
            provider_title: None,
            registry_item: None,
            modrinth_result: None,
            name: format!("Item {id}"),
            icon_url: None,
            description: None,
            content_type: "mod".into(),
            hero_image_url: None,
            author: None,
            categories: Vec::new(),
            downloads: None,
            follows: None,
            upvotes: None,
            downvotes: None,
            net_score: None,
            supported_versions: Vec::new(),
            source_page_url: None,
            technic_result: None,
            score: 0.0,
            score_breakdown: None,
            ranking_profile: None,
        }
    }

    fn test_descriptor() -> crate::providers::ProviderDescriptor {
        crate::providers::ProviderDescriptor {
            id: "acme.src/example".into(),
            title: "Example".into(),
            description: None,
            origin: crate::providers::ProviderOrigin::Plugin {
                plugin_id: "acme.src".into(),
            },
            content_types: vec!["mod".into()],
            filters: vec![],
            sorts: vec![],
            paginates: true,
            ranking: RankingProfile::default(),
            download_hosts: vec![],
            enabled: true,
            unavailable_reason: None,
        }
    }

    /// The cursor update for a provider that has nothing left upstream.
    fn exhausted() -> BTreeMap<String, ProviderCursor> {
        BTreeMap::from([(
            "modrinth".to_string(),
            ProviderCursor {
                offset: 100,
                has_more: false,
            },
        )])
    }

    fn modrinth_item() -> ModrinthSearchResult {
        ModrinthSearchResult {
            project_id: "modrinth-id".into(),
            slug: "example-project".into(),
            title: "Example Project".into(),
            description: "Description".into(),
            icon_url: Some("https://cdn.modrinth.com/icon.png".into()),
            author: "author".into(),
            categories: vec!["fabric".into()],
            downloads: 42,
            follows: 7,
            project_type: "mod".into(),
            date_created: None,
            date_modified: None,
            versions: vec!["1.21.1".into()],
            license: Some("MIT".into()),
            gallery: vec!["https://cdn.modrinth.com/gallery.png".into()],
            featured_gallery: Some("https://cdn.modrinth.com/featured.png".into()),
        }
    }

    #[test]
    fn normalizes_modrinth_project_types_for_browse() {
        assert_eq!(normalize_modrinth_content_type("modpack"), "pack");
        assert_eq!(
            normalize_modrinth_content_type("minecraft_java_server"),
            "server"
        );
        assert_eq!(normalize_modrinth_content_type("shader"), "shader");
    }

    #[test]
    fn normalizes_modrinth_presentation_without_extra_requests() {
        let item = item_from_modrinth(modrinth_item());
        assert_eq!(
            item.hero_image_url.as_deref(),
            Some("https://cdn.modrinth.com/featured.png")
        );
        assert_eq!(item.author.as_deref(), Some("author"));
        assert_eq!(item.downloads, Some(42));
        assert_eq!(item.supported_versions, vec!["1.21.1"]);
        assert_eq!(
            item.source_page_url.as_deref(),
            Some("https://modrinth.com/mod/example-project")
        );
    }

    #[test]
    fn rejects_non_https_presentation_urls() {
        let mut source = modrinth_item();
        source.featured_gallery = Some("http://cdn.modrinth.com/featured.png".into());
        source.gallery = vec!["file:///private/image.png".into()];
        let item = item_from_modrinth(source);
        assert!(item.hero_image_url.is_none());
    }

    #[tokio::test]
    async fn requested_pages_do_not_skip_cached_items() {
        let cache = new_cache();
        {
            let mut state = cache.write().await;
            state.query_key = "query-a".into();
            state.items = (0..95).map(item).collect();
            state.total = state.items.len();
        }
        let page_one = get_page(&cache, 1).await;
        assert_eq!(
            page_one.items.first().map(|i| i.id.as_str()),
            Some("item-20")
        );
        assert_eq!(
            page_one.items.last().map(|i| i.id.as_str()),
            Some("item-39")
        );
    }

    #[tokio::test]
    async fn stale_query_append_is_rejected() {
        let cache = new_cache();
        cache.write().await.query_key = "query-b".into();
        let appended = append_items(&cache, "query-a", vec![item(1)], exhausted(), 0.5).await;
        assert!(!appended);
        assert!(cache.read().await.items.is_empty());
    }

    fn scored(id: &str, source: &str, downloads: i64, follows: i64) -> BrowseItem {
        let mut base = item(0);
        base.id = id.to_string();
        base.name = id.to_string();
        base.source = source.to_string();
        base.downloads = Some(downloads);
        base.follows = Some(follows);
        base
    }

    #[test]
    fn merge_sorts_by_score_not_by_source() {
        // Regression: the old merge emitted every Modrinth hit ahead of every
        // curated leftover, so page 0 was all-Modrinth regardless of quality.
        let mut items = vec![
            scored("weak-modrinth", "modrinth", 5_000, 5),
            scored("strong-curated", "curated", 200_000_000, 40_000),
        ];
        score_and_sort(&mut items, 0.5);
        assert_eq!(items[0].id, "strong-curated");
        assert!(items[0].score > items[1].score);
    }

    #[test]
    fn curated_band_floor_keeps_weak_curated_mid_list() {
        let mut items = vec![
            scored("no-data-curated", "curated", 0, 0),
            scored("huge-modrinth", "modrinth", 250_000_000, 50_000),
            scored("tiny-modrinth", "modrinth", 100, 0),
        ];
        score_and_sort(&mut items, 0.5);
        // Strong uncurated outranks a curated item with no popularity data...
        assert_eq!(items[0].id, "huge-modrinth");
        // ...but that curated item still sits above weak uncurated content.
        assert_eq!(items[1].id, "no-data-curated");
        assert_eq!(items[2].id, "tiny-modrinth");
    }

    #[test]
    fn technic_items_get_a_namespaced_id() {
        let technic = TechnicSearchResult {
            slug: "complex-pixelmon".into(),
            title: "Complex Pixelmon".into(),
            description: "d".into(),
            installs: 1_582_592,
            likes: 1_730,
            author: Some("someone".into()),
            page_url: "https://www.technicpack.net/modpack/complex-pixelmon".into(),
            icon_url: None,
            tags: vec!["adventure".into()],
            tier: crate::technic::TechnicTier::Solder,
        };
        let merged = merge_items(vec![], vec![item_from_technic(technic)], 0.5);
        assert_eq!(merged.len(), 1);
        // Must not collide with a Modrinth project id or a registry item id.
        assert_eq!(merged[0].id, "technic:complex-pixelmon");
        assert_eq!(merged[0].source, "technic");
        assert_eq!(merged[0].content_type, "pack");
    }

    #[test]
    fn a_curated_row_absorbs_the_matching_modrinth_hit_but_not_other_providers() {
        let registry: RegistryItem = serde_json::from_value(serde_json::json!({
            "id": "curated-example",
            "name": "Curated Example",
            "content_type": "mod",
            "download_strategy": "github_release",
            "source_identifier": "example/example",
            "sha256": "",
            "upvotes": 0,
            "downvotes": 0,
            "net_score": 0,
            "velocity": 0.0,
            "status": "active",
            "is_immune": false,
            "immunity_reason": null,
            "allow_comments": true,
            "icon_url": null,
            "gallery_urls_json": null,
            "date_added": null,
            "compatible_versions_json": null,
            "description": null,
            "body_markdown": null,
            "page_url": null,
            "license_id": null,
            "source_updated_at": null,
            "modrinth_id": "modrinth-id"
        }))
        .unwrap();

        let plugin_hit = crate::providers::ProviderHit {
            summary: agora_plugin_api::provider::ProjectSummary {
                // Same raw id as the Modrinth project: a different provider's
                // project is a different thing, whatever its id looks like.
                id: "modrinth-id".into(),
                title: "Elsewhere".into(),
                content_type: "mod".into(),
                ..Default::default()
            },
            native: None,
        };
        let merged = merge_items(
            vec![registry],
            vec![
                item_from_modrinth(modrinth_item()),
                item_from_hit(&test_descriptor(), plugin_hit),
            ],
            0.5,
        );
        assert_eq!(merged.len(), 2);
        let curated = merged.iter().find(|i| i.source == "curated").unwrap();
        assert!(curated.provider_id.is_none());
        assert!(curated.modrinth_result.is_some());
        let plugin = merged.iter().find(|i| i.source != "curated").unwrap();
        assert_eq!(plugin.id, "provider:acme.src/example:modrinth-id");
        assert_eq!(plugin.provider_title.as_deref(), Some("Example"));
    }

    #[test]
    fn plugin_items_drop_non_https_images() {
        let hit = crate::providers::ProviderHit {
            summary: agora_plugin_api::provider::ProjectSummary {
                id: "p".into(),
                title: "P".into(),
                content_type: "mod".into(),
                icon_url: Some("http://example.com/i.png".into()),
                page_url: Some("javascript:alert(1)".into()),
                ..Default::default()
            },
            native: None,
        };
        let item = item_from_hit(&test_descriptor(), hit);
        assert!(item.icon_url.is_none());
        assert!(item.source_page_url.is_none());
    }

    #[test]
    fn split_chunk_holds_the_remainder() {
        let pile: Vec<BrowseItem> = (0..5).map(item).collect();
        let (shown, held) = split_chunk(pile, 2);
        assert_eq!(shown.len(), 2);
        assert_eq!(held.len(), 3);

        let short: Vec<BrowseItem> = (0..2).map(item).collect();
        let (shown, held) = split_chunk(short, 10);
        assert_eq!(shown.len(), 2);
        assert!(held.is_empty());
    }

    #[tokio::test]
    async fn appending_never_reorders_already_shown_items() {
        let cache = new_cache();
        {
            let mut c = cache.write().await;
            c.query_key = "q".into();
            c.items = vec![scored("shown-a", "modrinth", 1_000, 1)];
        }
        let before = cache.read().await.items[0].id.clone();
        // A far stronger item arrives later; it must land BELOW what is shown.
        let strong = scored("late-strong", "curated", 250_000_000, 50_000);
        assert!(append_items(&cache, "q", vec![strong], exhausted(), 0.5).await);
        let items = &cache.read().await.items;
        assert_eq!(items[0].id, before, "an already-displayed item moved");
        assert_eq!(items[1].id, "late-strong");
    }

    #[tokio::test]
    async fn exhausting_upstream_flushes_the_buffer() {
        let cache = new_cache();
        {
            let mut c = cache.write().await;
            c.query_key = "q".into();
            c.buffer = (0..30).map(item).collect();
        }
        // has_more = false means upstream is done: nothing may be left behind.
        assert!(append_items(&cache, "q", vec![], exhausted(), 0.5).await);
        let c = cache.read().await;
        assert!(c.buffer.is_empty(), "buffered items were stranded");
        assert_eq!(c.items.len(), 30);
    }

    #[tokio::test]
    async fn drain_buffer_serves_pages_without_fetching() {
        let cache = new_cache();
        {
            let mut c = cache.write().await;
            c.query_key = "q".into();
            c.items = (0..PAGE_SIZE).map(item).collect();
            c.buffer = (PAGE_SIZE..PAGE_SIZE * 3).map(item).collect();
        }
        assert!(drain_buffer(&cache, "q", PAGE_SIZE * 2).await);
        let c = cache.read().await;
        assert!(c.items.len() >= PAGE_SIZE * 2);
        assert_eq!(c.items.len() + c.buffer.len(), PAGE_SIZE * 3);
    }

    #[tokio::test]
    async fn drain_buffer_rejects_a_stale_query() {
        let cache = new_cache();
        cache.write().await.query_key = "q".into();
        assert!(!drain_buffer(&cache, "other", PAGE_SIZE).await);
    }

    /// Walks the real page sequence: initial load, then repeated load-more,
    /// mirroring what `browse_load_more` does around the cache.
    #[tokio::test]
    async fn paging_walks_every_item_without_stalling() {
        let cache = new_cache();
        // 6 curated (fetched in full) + 20 from a paginating provider.
        let seeded: Vec<BrowseItem> = (0..26).map(item).collect();
        load_initial(
            &cache,
            "q".into(),
            Vec::new(),
            BrowseFilters::default(),
            BTreeMap::from([(
                "modrinth".to_string(),
                ProviderCursor {
                    offset: PAGE_SIZE as u32,
                    has_more: true,
                },
            )]),
        )
        .await;
        {
            let mut c = cache.write().await;
            c.items = seeded[..PAGE_SIZE].to_vec();
            c.buffer = seeded[PAGE_SIZE..].to_vec();
            c.total = seeded.len();
        }

        let page0 = get_page(&cache, 0).await;
        assert_eq!(page0.items.len(), PAGE_SIZE);
        assert!(page0.has_more, "page 0 must offer more");

        // Page 1 with upstream exhausted: only the 6 buffered items remain.
        assert!(append_items(&cache, "q", vec![], exhausted(), 0.5).await);
        assert!(drain_buffer(&cache, "q", PAGE_SIZE * 2).await);
        let page1 = get_page(&cache, 1).await;
        assert_eq!(page1.items.len(), 6, "buffered remainder must be served");
        assert!(!page1.has_more, "nothing left after the buffer drains");

        let c = cache.read().await;
        assert!(c.buffer.is_empty());
        assert_eq!(c.items.len(), 26, "every item must be reachable");
    }

    #[tokio::test]
    async fn append_deduplicates_by_source_and_id() {
        let cache = new_cache();
        cache.write().await.query_key = "query-a".into();
        let duplicate = item(1);
        assert!(
            append_items(
                &cache,
                "query-a",
                vec![duplicate.clone(), duplicate],
                exhausted(),
                0.5,
            )
            .await
        );
        assert_eq!(cache.read().await.items.len(), 1);
    }
}
