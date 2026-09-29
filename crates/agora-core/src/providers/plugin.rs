//! A plugin's content provider, behind the same trait as Agora's own.
//!
//! Every answer a plugin gives is untrusted data. It is parsed into the
//! contract types and validated against the contract's bounds before anything
//! else in Agora sees it; a malformed page is an error attributed to the
//! plugin, never a partially-rendered list.

use super::{
    network_unavailable_reason, ContentProvider, ProviderDescriptor, ProviderHit, ProviderOrigin,
    ProviderPage,
};
use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::plugins::PluginService;
use agora_plugin_api::manifest::PluginId;
use agora_plugin_api::provider::{
    InstallPlan, ProjectDetail, ProjectRequest, ProviderContribution, ResolveRequest,
    SearchRequest, SearchResponse, VersionsRequest, VersionsResponse, MAX_PAGE_SIZE,
};
use agora_plugin_api::PluginError;
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use std::sync::Arc;

pub struct PluginProvider {
    ctx: Ctx,
    service: PluginService,
    plugin_id: PluginId,
    contribution: ProviderContribution,
    declared_hosts: Vec<String>,
    runnable: bool,
}

impl PluginProvider {
    /// One provider per `contentProviders` entry of every installed plugin
    /// that holds `content:provide`. A disabled plugin's providers are listed
    /// as switched off, so they can be switched back on from the same place.
    pub fn discover(ctx: &Ctx, service: &PluginService) -> Vec<Arc<dyn ContentProvider>> {
        service
            .content_providers()
            .into_iter()
            .map(|found| {
                Arc::new(PluginProvider {
                    ctx: ctx.clone(),
                    service: service.clone(),
                    plugin_id: found.plugin_id,
                    contribution: found.contribution,
                    declared_hosts: found.declared_hosts,
                    runnable: found.runnable,
                }) as Arc<dyn ContentProvider>
            })
            .collect()
    }

    fn qualified_id(&self) -> String {
        self.plugin_id.qualify(&self.contribution.id)
    }

    /// Call an export on a blocking thread and parse its answer.
    async fn call<T: DeserializeOwned>(
        &self,
        export: &str,
        args: serde_json::Value,
    ) -> LauncherResult<T> {
        let service = self.service.clone();
        let plugin_id = self.plugin_id.clone();
        let export_name = export.to_string();
        let value = tokio::task::spawn_blocking(move || {
            service.call_provider(&plugin_id, &export_name, args)
        })
        .await
        .map_err(|e| LauncherError::Generic {
            code: "ERR_PROVIDER_TASK".into(),
            message: format!("The provider call did not complete: {e}"),
        })?
        .map_err(|e| self.plugin_error(e))?;
        serde_json::from_value(value).map_err(|e| LauncherError::Generic {
            code: "ERR_PROVIDER_RESPONSE".into(),
            message: format!(
                "{} returned something that is not a valid `{export}` result: {e}",
                self.contribution.title
            ),
        })
    }

    fn plugin_error(&self, error: PluginError) -> LauncherError {
        LauncherError::Generic {
            code: format!("ERR_PROVIDER_{:?}", error.code).to_uppercase(),
            message: format!("{}: {}", self.contribution.title, error.message),
        }
    }

    fn checked(&self, result: agora_plugin_api::error::PluginResult<()>) -> LauncherResult<()> {
        result.map_err(|e| self.plugin_error(e))
    }

    fn to_args<T: serde::Serialize>(request: &T) -> serde_json::Value {
        serde_json::to_value(request).unwrap_or(serde_json::Value::Null)
    }
}

#[async_trait]
impl ContentProvider for PluginProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.qualified_id(),
            title: self.contribution.title.clone(),
            description: self.contribution.description.clone(),
            origin: ProviderOrigin::Plugin {
                plugin_id: self.plugin_id.to_string(),
            },
            content_types: self.contribution.content_types.clone(),
            filters: self.contribution.filters.clone(),
            sorts: self.contribution.sorts.clone(),
            paginates: self.contribution.paginates,
            ranking: self.contribution.ranking.clone(),
            download_hosts: self.declared_hosts.clone(),
            // Switching a plugin provider off *is* disabling its plugin, so
            // there is one switch rather than two that can disagree.
            enabled: self.runnable,
            // A provider that talks to the network cannot work while plugin
            // network access is off; say so instead of failing every search.
            unavailable_reason: if self.declared_hosts.is_empty() {
                network_unavailable_reason(&self.ctx, None)
            } else {
                network_unavailable_reason(&self.ctx, Some("network_plugins_enabled"))
            },
        }
    }

    async fn categories(
        &self,
    ) -> LauncherResult<Vec<agora_plugin_api::provider::CategoryDefinition>> {
        Ok(self.contribution.categories.clone())
    }

    async fn search(&self, mut request: SearchRequest) -> LauncherResult<ProviderPage> {
        request.limit = request.limit.min(MAX_PAGE_SIZE as u32);
        // Only the provider's own declared filter values are forwarded, and
        // only when they are ones it declared: Browse state from another
        // provider must not leak into this one's request.
        request.filters.retain(|id, values| {
            self.contribution
                .filters
                .iter()
                .any(|f| &f.id == id && f.accepts(values))
        });
        let response: SearchResponse = self
            .call(&self.contribution.exports.search, Self::to_args(&request))
            .await?;
        self.checked(response.validate())?;
        Ok(ProviderPage {
            total: response.total,
            has_more: response.has_more && self.contribution.paginates,
            hits: response
                .items
                .into_iter()
                .map(|summary| ProviderHit {
                    summary,
                    native: None,
                })
                .collect(),
        })
    }

    async fn project(&self, project_id: &str) -> LauncherResult<ProjectDetail> {
        let Some(export) = self.contribution.exports.project.clone() else {
            return Err(LauncherError::Generic {
                code: "ERR_PROVIDER_NO_DETAIL".into(),
                message: format!("{} does not offer project pages.", self.contribution.title),
            });
        };
        let detail: ProjectDetail = self
            .call(
                &export,
                Self::to_args(&ProjectRequest {
                    project_id: project_id.to_string(),
                }),
            )
            .await?;
        self.checked(detail.validate())?;
        Ok(detail)
    }

    async fn versions(&self, request: VersionsRequest) -> LauncherResult<VersionsResponse> {
        let response: VersionsResponse = self
            .call(&self.contribution.exports.versions, Self::to_args(&request))
            .await?;
        self.checked(response.validate())?;
        Ok(response)
    }

    async fn resolve(&self, request: ResolveRequest) -> LauncherResult<InstallPlan> {
        let plan: InstallPlan = self
            .call(&self.contribution.exports.resolve, Self::to_args(&request))
            .await?;
        self.checked(plan.validate())?;
        Ok(plan)
    }
}
