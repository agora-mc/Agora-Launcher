use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::modrinth::ModrinthSearchResult;
use crate::registry::RegistryItem;

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
    pub source: String, // "curated" | "modrinth"
    pub registry_item: Option<RegistryItem>,
    pub modrinth_result: Option<ModrinthSearchResult>,
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
    pub modrinth_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowseCache {
    /// Immutable identity of the filters that produced this cache.
    pub query_key: String,
    pub items: Vec<BrowseItem>,
    pub total: usize,
    pub filters: BrowseFilters,
    pub modrinth_offset: usize,
    pub has_more_modrinth: bool,
}

impl Default for BrowseCache {
    fn default() -> Self {
        Self {
            query_key: String::new(),
            items: Vec::new(),
            total: 0,
            filters: BrowseFilters::default(),
            modrinth_offset: 0,
            has_more_modrinth: true,
        }
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
    }
}

/// Merge registry items and Modrinth results, deduplicating by modrinth_id.
pub fn merge_items(
    registry_items: Vec<RegistryItem>,
    modrinth_results: Vec<ModrinthSearchResult>,
) -> Vec<BrowseItem> {
    let mut registry_by_modrinth_id = HashMap::new();
    for ri in &registry_items {
        if let Some(ref mid) = ri.modrinth_id {
            registry_by_modrinth_id.insert(mid.clone(), ri.clone());
        }
    }

    let mut matched_ids = std::collections::HashSet::new();
    let mut merged = Vec::new();

    for mr in modrinth_results {
        if let Some(matched) = registry_by_modrinth_id.get(&mr.project_id) {
            matched_ids.insert(matched.id.clone());
            let pres = normalized_presentation(Some(matched), Some(&mr));
            merged.push(BrowseItem {
                id: matched.id.clone(),
                source: "curated".to_string(),
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
            });
        } else {
            merged.push(item_from_modrinth(mr));
        }
    }

    for ri in registry_items {
        if !matched_ids.contains(&ri.id) {
            let pres = normalized_presentation(Some(&ri), None);
            merged.push(BrowseItem {
                id: ri.id.clone(),
                source: "curated".to_string(),
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
            });
        }
    }

    merged
}

/// Load the first page of browse results into the cache.
pub async fn load_initial(
    cache: &SharedBrowseCache,
    query_key: String,
    registry_items: Vec<RegistryItem>,
    modrinth_results: Vec<ModrinthSearchResult>,
    filters: BrowseFilters,
    modrinth_offset: usize,
    has_more_modrinth: bool,
) {
    let merged = merge_items(registry_items, modrinth_results);
    let total = merged.len();
    let mut c = cache.write().await;
    c.query_key = query_key;
    c.items = merged;
    c.total = total;
    c.filters = filters;
    c.modrinth_offset = modrinth_offset;
    c.has_more_modrinth = has_more_modrinth;
}

/// Append more Modrinth items only when the cache still belongs to the
/// expected query. Returns false when a newer query replaced the cache.
pub async fn append_items(
    cache: &SharedBrowseCache,
    expected_query_key: &str,
    new_items: Vec<BrowseItem>,
    new_offset: usize,
    has_more: bool,
) -> bool {
    let mut c = cache.write().await;
    if c.query_key != expected_query_key {
        return false;
    }
    let mut existing_ids: std::collections::HashSet<(String, String)> = c
        .items
        .iter()
        .map(|i| (i.source.clone(), i.id.clone()))
        .collect();
    for item in new_items {
        if existing_ids.insert((item.source.clone(), item.id.clone())) {
            c.items.push(item);
        }
    }
    c.total = c.items.len();
    c.modrinth_offset = new_offset;
    c.has_more_modrinth = has_more;
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
        has_more: end < c.items.len(),
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
        }
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
            state.has_more_modrinth = false;
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
        let appended = append_items(&cache, "query-a", vec![item(1)], PAGE_SIZE, false).await;
        assert!(!appended);
        assert!(cache.read().await.items.is_empty());
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
                PAGE_SIZE,
                false,
            )
            .await
        );
        assert_eq!(cache.read().await.items.len(), 1);
    }
}
