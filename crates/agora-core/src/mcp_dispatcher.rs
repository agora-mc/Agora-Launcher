//! Transport-free MCP dispatcher — core-owned tool definitions,
//! JSON-RPC-neutral method dispatch, read-only tool implementations
//! via `Ctx`, and approval-grant checks from local_state.db.
//!
//! Constraint: no HTTP parsing, bearer tokens, TCP/SSE sessions,
//! or Tauri types.

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::instance_service::InstanceService;
use crate::models::InstanceManifest;

// ---------------------------------------------------------------------------
// McpDispatcher struct — ergonomic wrapper for transport handlers
// ---------------------------------------------------------------------------

/// Cloneable MCP dispatcher wrapping a shared [`Ctx`].
///
/// Provides named methods for each MCP method so that callers (CLI stdio,
/// desktop portable routing) can avoid repeating the method-name dispatch
/// pattern.  Delegates to the free functions below; see those for docs.
#[derive(Clone)]
pub struct McpDispatcher {
    ctx: Ctx,
}

impl McpDispatcher {
    pub fn new(ctx: Ctx) -> Self {
        Self { ctx }
    }

    pub fn initialize(&self) -> serde_json::Value {
        handle_mcp_method(&self.ctx, "initialize", None)
    }

    pub fn list_tools(&self) -> serde_json::Value {
        handle_mcp_method(&self.ctx, "tools/list", None)
    }

    pub fn call_tool(&self, name: &str, arguments: &serde_json::Value) -> serde_json::Value {
        let params = serde_json::json!({ "name": name, "arguments": arguments });
        handle_mcp_method(&self.ctx, "tools/call", Some(&params))
    }

    pub fn list_resources(&self) -> serde_json::Value {
        handle_mcp_method(&self.ctx, "resources/list", None)
    }

    pub fn read_resource(&self, uri: &str) -> serde_json::Value {
        let params = serde_json::json!({ "uri": uri });
        handle_mcp_method(&self.ctx, "resources/read", Some(&params))
    }

    /// Dispatch an arbitrary method by name (same as calling the free
    /// [`handle_mcp_method`]).
    pub fn handle_method(
        &self,
        method: &str,
        params: Option<&serde_json::Value>,
    ) -> serde_json::Value {
        handle_mcp_method(&self.ctx, method, params)
    }
}

// ---------------------------------------------------------------------------
// MCP method dispatch
// ---------------------------------------------------------------------------

/// Dispatch a JSON-RPC MCP method and return the result payload.
///
/// The returned `serde_json::Value` is the method-specific result object
/// (not a full JSON-RPC envelope). The caller is responsible for wrapping it
/// in a `JsonRpcResponse` and handling HTTP/SSE/stdio framing.
pub fn handle_mcp_method(
    ctx: &Ctx,
    method: &str,
    params: Option<&serde_json::Value>,
) -> serde_json::Value {
    match method {
        "initialize" => serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "resources": {}
            },
            "serverInfo": {
                "name": "agora",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
        "tools/list" => serde_json::json!({
            "tools": tool_definitions()
        }),
        "tools/call" => {
            let p = params.unwrap_or(&serde_json::Value::Null);
            let tool_name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tool_params = p.get("arguments").unwrap_or(&serde_json::Value::Null);
            handle_tool_call(ctx, tool_name, tool_params)
        }
        "resources/list" => serde_json::json!({
            "resources": resource_definitions()
        }),
        "resources/read" => {
            let p = params.unwrap_or(&serde_json::Value::Null);
            let uri = p.get("uri").and_then(|v| v.as_str()).unwrap_or("");
            handle_read_resource(ctx, uri)
        }
        _ => serde_json::json!({
            "error": {
                "code": -32601,
                "message": format!("Unknown method: {method}"),
            }
        }),
    }
}

// ---------------------------------------------------------------------------
// Tool metadata / definitions
// ---------------------------------------------------------------------------

/// Return the list of MCP tool definitions as JSON values.
pub fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "list_instances",
            "description": "List all Minecraft instances managed by Agora, including their IDs, names, and loader configurations.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        serde_json::json!({
            "name": "list_instance_mods",
            "description": "List all installed mods for a specific instance, including filenames, versions, sources, and dependency information.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "instance_id": {
                        "type": "string",
                        "description": "The instance ID to list mods for."
                    }
                },
                "required": ["instance_id"]
            }
        }),
        serde_json::json!({
            "name": "read_mod_manifest",
            "description": "Fetch curated metadata for a specific mod from the local SQLite registry, including curator notes, categories, compatibility data, and license info.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mod_id": {
                        "type": "string",
                        "description": "The registry ID of the mod (e.g. 'sodium', 'iris')."
                    }
                },
                "required": ["mod_id"]
            }
        }),
        serde_json::json!({
            "name": "get_system_context",
            "description": "Return a markdown summary of the current Agora system state, including instances, installed mods, and recent crashes.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        serde_json::json!({
            "name": "search_crash_signatures",
            "description": "Search the curated crash signature database for patterns matching the provided crash text. Returns matching signatures and fix hints.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "crash_text": {
                        "type": "string",
                        "description": "The crash log text to search against."
                    }
                },
                "required": ["crash_text"]
            }
        }),
        serde_json::json!({
            "name": "search_knowledge_base",
            "description": "Search the curated registry for mods matching a natural-language query. Uses LIKE matching across mod names and descriptions in the local SQLite database.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural-language search string, e.g. 'performance rendering optimization' or 'magic mod'."
                    }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "disable_mod",
            "description": "Disable a mod in an instance by renaming its .jar file to .jar.disabled. Destructive — requires approval.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "instance_id": {
                        "type": "string",
                        "description": "The instance ID."
                    },
                    "filename": {
                        "type": "string",
                        "description": "The mod filename to disable."
                    }
                },
                "required": ["instance_id", "filename"]
            }
        }),
        serde_json::json!({
            "name": "enable_mod",
            "description": "Re-enable a previously disabled mod in an instance by renaming its .jar.disabled file back to .jar. Destructive — requires approval.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "instance_id": {
                        "type": "string",
                        "description": "The instance ID."
                    },
                    "filename": {
                        "type": "string",
                        "description": "The mod filename to re-enable."
                    }
                },
                "required": ["instance_id", "filename"]
            }
        }),
        serde_json::json!({
            "name": "read_latest_crash",
            "description": "Read the most recent crash report or log for an instance. Returns the last 200 lines of the newest crash file.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "instance_id": {
                        "type": "string",
                        "description": "The instance ID to read crash reports for."
                    }
                },
                "required": ["instance_id"]
            }
        }),
        serde_json::json!({
            "name": "suggest_mod_incompatibility",
            "description": "Analyze crash text against installed mods in an instance and return ranked suspect mods that may be causing the crash.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "instance_id": {
                        "type": "string",
                        "description": "The instance ID."
                    },
                    "crash_text": {
                        "type": "string",
                        "description": "The crash log text to analyze."
                    }
                },
                "required": ["instance_id", "crash_text"]
            }
        }),
    ]
}

// ---------------------------------------------------------------------------
// Tool call dispatch
// ---------------------------------------------------------------------------

fn handle_tool_call(ctx: &Ctx, tool_name: &str, params: &serde_json::Value) -> serde_json::Value {
    let get_str = |key: &str| -> Option<String> {
        params
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let instance_id = get_str("instance_id").unwrap_or_default();

    match tool_name {
        "list_instances" => {
            let result = handle_list_instances(ctx);
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&result).unwrap_or_default(),
                }],
                "isError": result.get("error").is_some(),
            })
        }
        "list_instance_mods" => {
            let result = handle_list_instance_mods(ctx, &instance_id);
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&result).unwrap_or_default(),
                }],
                "isError": result.get("error").is_some(),
            })
        }
        "read_mod_manifest" => {
            let mod_id = get_str("mod_id").unwrap_or_default();
            let result = handle_read_mod_manifest(ctx, &mod_id);
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&result).unwrap_or_default(),
                }],
                "isError": result.get("isError").map(|v| v.as_bool().unwrap_or(false)).unwrap_or(false),
            })
        }
        "get_system_context" => {
            let md = build_system_context(ctx);
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": md,
                }],
                "isError": false,
            })
        }
        "search_crash_signatures" => {
            let crash_text = get_str("crash_text").unwrap_or_default();
            let result = handle_search_crash_signatures(ctx, &crash_text);
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&result).unwrap_or_default(),
                }],
                "isError": false,
            })
        }
        "search_knowledge_base" => {
            let query = get_str("query").unwrap_or_default();
            let result = handle_search_knowledge_base(ctx, &query);
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string(&result).unwrap_or_default(),
                }],
                "isError": result.get("isError").map(|v| v.as_bool().unwrap_or(false)).unwrap_or(false),
            })
        }
        "disable_mod" => {
            let filename = get_str("filename").unwrap_or_default();
            if let Err(e) = check_approval(ctx, "disable_mod", &instance_id, true) {
                return serde_json::json!({
                    "content": [{ "type": "text", "text": format!("Approval denied: {}", e) }],
                    "isError": true,
                });
            }
            let service = crate::crash_service::CrashService::new(ctx.clone());
            match service.disable_mod(&instance_id, &filename) {
                Ok(()) => serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Mod {} disabled in instance {}. Restart the game to apply.", filename, instance_id),
                    }],
                    "isError": false,
                }),
                Err(e) => serde_json::json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true,
                }),
            }
        }
        "enable_mod" => {
            let filename = get_str("filename").unwrap_or_default();
            if let Err(e) = check_approval(ctx, "enable_mod", &instance_id, true) {
                return serde_json::json!({
                    "content": [{ "type": "text", "text": format!("Approval denied: {}", e) }],
                    "isError": true,
                });
            }
            let service = crate::crash_service::CrashService::new(ctx.clone());
            match service.enable_mod(&instance_id, &filename) {
                Ok(()) => serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Mod {} re-enabled in instance {}. Restart the game to apply.", filename, instance_id),
                    }],
                    "isError": false,
                }),
                Err(e) => serde_json::json!({
                    "content": [{ "type": "text", "text": e.to_string() }],
                    "isError": true,
                }),
            }
        }
        "suggest_mod_incompatibility" => {
            let crash_text = get_str("crash_text").unwrap_or_default();
            let service = crate::crash_service::CrashService::new(ctx.clone());
            match service.suggest_mod_incompatibility(&instance_id, &crash_text) {
                Ok(suspects) => {
                    let results: Vec<serde_json::Value> = suspects
                        .into_iter()
                        .map(|s| {
                            serde_json::json!({
                                "mod_id": s.mod_id,
                                "filename": s.filename,
                                "total_score": s.total_score,
                                "is_dependent_of": s.is_dependent_of,
                                "breakdown": s.breakdown,
                            })
                        })
                        .collect();
                    serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string(&serde_json::json!({ "suspects": results }))
                                .unwrap_or_default(),
                        }],
                        "isError": false,
                    })
                }
                Err(e) => serde_json::json!({
                    "content": [{ "type": "text", "text": format!("{}", e) }],
                    "isError": true,
                }),
            }
        }
        "read_latest_crash" => {
            let service = crate::crash_service::CrashService::new(ctx.clone());
            let reports = match service.list_reports(&instance_id) {
                Ok(r) => r,
                Err(e) => {
                    return serde_json::json!({
                        "content": [{"type": "text", "text": format!("Error listing crash reports: {}", e)}],
                        "isError": true,
                    })
                }
            };
            let newest = match reports.first() {
                Some(r) => r.filename.clone(),
                None => {
                    return serde_json::json!({
                        "content": [{"type": "text", "text": format!("No crash reports found for instance '{}'", instance_id)}],
                        "isError": false,
                    })
                }
            };
            let full = match service.read_crash_log(&instance_id, &newest) {
                Ok(t) => t,
                Err(e) => {
                    return serde_json::json!({
                        "content": [{"type": "text", "text": format!("Error reading crash log: {}", e)}],
                        "isError": true,
                    })
                }
            };
            let lines: Vec<&str> = full.lines().collect();
            let start = if lines.len() > 200 {
                lines.len() - 200
            } else {
                0
            };
            let tail: Vec<&str> = lines[start..].to_vec();
            serde_json::json!({
                "content": [{"type": "text", "text": tail.join("\n")}],
                "isError": false,
                "filename": newest,
                "total_lines": lines.len(),
            })
        }
        _ => serde_json::json!({
            "content": [{
                "type": "text",
                "text": format!("Tool '{}' not found", tool_name),
            }],
            "isError": true,
        }),
    }
}

// ---------------------------------------------------------------------------
// Tool implementations (read-only, via Ctx)
// ---------------------------------------------------------------------------

fn handle_list_instances(ctx: &Ctx) -> serde_json::Value {
    let service = InstanceService::new(ctx.clone());
    match service.list() {
        Ok(rows) => {
            let items: Vec<serde_json::Value> = rows
                .into_iter()
                .map(|r| {
                    serde_json::json!({
                        "instance_id": r.instance_id,
                        "name": r.name,
                        "minecraft_version": r.minecraft_version,
                        "loader": r.loader,
                        "loader_version": r.loader_version,
                    })
                })
                .collect();
            serde_json::json!({ "instances": items })
        }
        Err(e) => serde_json::json!({ "error": "ERR_MCP_INTERNAL", "message": e.to_string() }),
    }
}

fn handle_list_instance_mods(ctx: &Ctx, instance_id: &str) -> serde_json::Value {
    let manifest = match read_instance_manifest(ctx, instance_id) {
        Ok(m) => m,
        Err(e) => {
            return serde_json::json!({
                "error": "ERR_MCP_INTERNAL",
                "message": e.to_string(),
            });
        }
    };
    match manifest {
        Some(m) => {
            let mods: Vec<serde_json::Value> = m
                .mods
                .into_iter()
                .map(|mod_| {
                    serde_json::json!({
                        "filename": mod_.filename,
                        "version": mod_.version,
                        "source": mod_.source,
                        "mod_jar_id": mod_.mod_jar_id,
                        "depends_on": mod_.depends_on,
                        "optional_deps": mod_.optional_deps,
                        "java_packages": mod_.java_packages,
                    })
                })
                .collect();
            serde_json::json!({ "mods": mods })
        }
        None => serde_json::json!({ "mods": [] }),
    }
}

fn handle_read_mod_manifest(ctx: &Ctx, mod_id: &str) -> serde_json::Value {
    let conn = match open_registry(ctx) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "content": [{ "type": "text", "text": format!("Could not open registry: {e}") }],
                "isError": true,
            })
        }
    };
    let item = match crate::registry::get_item_by_id(&conn, mod_id) {
        Ok(Some(i)) => i,
        Ok(None) => {
            return serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Mod '{mod_id}' not found in curated registry"),
                }],
                "isError": true,
            })
        }
        Err(e) => {
            return serde_json::json!({
                "content": [{ "type": "text", "text": format!("Registry query error: {e}") }],
                "isError": true,
            })
        }
    };
    serde_json::json!({
        "id": item.id,
        "name": item.name,
        "content_type": item.content_type,
        "download_strategy": item.download_strategy,
        "source_identifier": item.source_identifier,
        "download_sources": item.download_sources(),
        "sha256": item.sha256,
        "license_id": item.license_id,
        "description": item.description,
        "body_markdown": item.body_markdown,
        "page_url": item.page_url,
        "icon_url": item.icon_url,
        "upvotes": item.upvotes,
        "downvotes": item.downvotes,
        "net_score": item.net_score,
        "velocity": item.velocity,
        "status": item.status,
        "is_immune": item.is_immune,
        "immunity_reason": item.immunity_reason,
        "date_added": item.date_added,
        "compatible_versions_json": item.compatible_versions_json,
        "modrinth_id": item.modrinth_id,
    })
}

fn build_system_context(ctx: &Ctx) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push("# Agora System Context".to_string());
    lines.push(String::new());

    let service = InstanceService::new(ctx.clone());

    // Instances
    lines.push("## Instances".to_string());
    match service.list() {
        Ok(rows) => {
            if rows.is_empty() {
                lines.push("No instances configured.".to_string());
            } else {
                for row in &rows {
                    lines.push(format!(
                        "- **{}** (`{}`) — Minecraft {}, Loader: {} {}",
                        row.name,
                        row.instance_id,
                        row.minecraft_version,
                        row.loader,
                        row.loader_version
                    ));
                }
            }
        }
        Err(e) => lines.push(format!("Error listing instances: {e}")),
    }
    lines.push(String::new());

    // Installed mods
    lines.push("## Installed Mods".to_string());
    let instance_rows = match service.list() {
        Ok(r) => r,
        Err(e) => {
            lines.push(format!("Error listing instances: {e}"));
            return lines.join("\n");
        }
    };
    let mut total_mods = 0usize;
    for row in &instance_rows {
        let manifest = read_instance_manifest(ctx, &row.instance_id);
        match manifest {
            Ok(Some(m)) => {
                lines.push(format!("- **{}** — {} mods", row.name, m.mods.len()));
                total_mods += m.mods.len();
                for mod_ in &m.mods {
                    let ver = mod_.version.as_deref().unwrap_or("unknown");
                    lines.push(format!(
                        "  - {} v{} (source: {})",
                        mod_.filename, ver, mod_.source
                    ));
                }
            }
            Ok(None) => lines.push(format!("- **{}** — manifest not found", row.name)),
            Err(_) => lines.push(format!("- **{}** — could not read manifest", row.name)),
        }
    }
    if total_mods == 0 && instance_rows.is_empty() {
        lines.push("No installed mods.".to_string());
    }
    lines.push(String::new());

    // Recent crashes
    lines.push("## Recent Crashes".to_string());
    let mut found_crashes = false;
    for row in &instance_rows {
        let instance_dir = match ctx.paths.instance_dir(&row.instance_id) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let crash_dir = instance_dir.join("crash-reports");
        if crash_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&crash_dir) {
                let mut crash_files: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let name = e.file_name();
                        let name = name.to_string_lossy();
                        name.ends_with(".log") || name.ends_with(".txt")
                    })
                    .collect();
                crash_files.sort_by(|a, b| {
                    let ma = a.metadata().and_then(|m| m.modified()).ok();
                    let mb = b.metadata().and_then(|m| m.modified()).ok();
                    mb.cmp(&ma)
                });
                for entry in crash_files.iter().take(3) {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    lines.push(format!("- {fname} ({size} bytes)"));
                    found_crashes = true;
                }
            }
        }
    }
    if !found_crashes {
        lines.push("No recent crash reports found.".to_string());
    }

    lines.join("\n")
}

fn handle_search_crash_signatures(_ctx: &Ctx, crash_text: &str) -> serde_json::Value {
    let result = crate::crash_diagnostics::triage(crash_text);
    if result.matched {
        serde_json::json!({
            "matches": [{
                "name": result.signature_name,
                "fix_hint": result.solution_markdown,
            }]
        })
    } else {
        serde_json::json!({ "matches": [] })
    }
}

fn handle_search_knowledge_base(ctx: &Ctx, query: &str) -> serde_json::Value {
    let conn = match open_registry(ctx) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "content": [{ "type": "text", "text": format!("Could not open registry: {e}") }],
                "isError": true,
            })
        }
    };
    let like_pattern = format!("%{query}%");
    let sql = "SELECT id, name, content_type, description \
               FROM registry_items \
               WHERE (description IS NOT NULL AND description LIKE ?1) \
                  OR (name LIKE ?1) \
               LIMIT 5";
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => {
            return serde_json::json!({
                "content": [{ "type": "text", "text": format!("Query prepare error: {e}") }],
                "isError": true,
            })
        }
    };
    let rows = match stmt.query_map(
        [&like_pattern],
        |row| -> rusqlite::Result<serde_json::Value> {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "content_type": row.get::<_, String>(2)?,
                "description": row.get::<_, Option<String>>(3)?,
            }))
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            return serde_json::json!({
                "content": [{ "type": "text", "text": format!("Query error: {e}") }],
                "isError": true,
            })
        }
    };
    let mut results: Vec<serde_json::Value> = Vec::new();
    for v in rows.flatten() {
        results.push(v);
    }
    serde_json::json!({
        "results": results,
        "query": query,
    })
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

pub fn resource_definitions() -> Vec<serde_json::Value> {
    vec![serde_json::json!({
        "uri": "system_context.md",
        "name": "System Context",
        "mimeType": "text/markdown",
        "description": "Current Agora system state",
    })]
}

fn handle_read_resource(ctx: &Ctx, uri: &str) -> serde_json::Value {
    if uri == "system_context.md" {
        let md = build_system_context(ctx);
        serde_json::json!({
            "contents": [{
                "uri": "system_context.md",
                "mimeType": "text/markdown",
                "text": md,
            }]
        })
    } else {
        serde_json::json!({
            "error": {
                "code": -32602,
                "message": format!("Unknown resource URI: {uri}"),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Approval check (parameterized reads from local_state.db)
// ---------------------------------------------------------------------------

/// Decision result for the pure approval logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalResult {
    Allowed,
    Denied,
}

/// Pure approval decision: given a stored grant state and whether the tool
/// is destructive, decide if the call is allowed.
///
/// Read-only tools (`is_destructive = false`) are always allowed (safe default).
/// Destructive tools require an explicit grant.
///
/// The caller is responsible for checking `expires_at` before calling this
/// function — expired grants should already be converted to `None`.
pub fn check_approval_grant(state: Option<&str>, is_destructive: bool) -> ApprovalResult {
    if !is_destructive {
        return ApprovalResult::Allowed;
    }
    match state {
        Some("always_allow") | Some("session") => ApprovalResult::Allowed,
        Some("always_deny") => ApprovalResult::Denied,
        None => ApprovalResult::Denied,
        Some(_) => ApprovalResult::Denied,
    }
}

/// Parameterized approval check from `local_state.db`.
///
/// Opens the local state DB via `ctx.paths.local_state_db()` and queries the
/// `mcp_approval_grants` table. Read-only tools always pass.
pub fn check_approval(
    ctx: &Ctx,
    tool_name: &str,
    instance_id: &str,
    is_destructive: bool,
) -> LauncherResult<()> {
    if !is_destructive {
        return Ok(());
    }

    let db_path = ctx.paths.local_state_db();
    let conn = crate::db::local_state_connection(&db_path).map_err(|_| LauncherError::Generic {
        code: "ERR_LOCAL_STATE_FAILED".into(),
        message: "Failed to open local state database for approval check.".into(),
    })?;

    let mut stmt = conn
        .prepare(
            "SELECT state, expires_at FROM mcp_approval_grants \
             WHERE tool_name = ?1 AND instance_id = ?2",
        )
        .map_err(|_| LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: "Failed to prepare approval query.".into(),
        })?;

    let row: Option<(String, Option<String>)> = match stmt
        .query_row([tool_name, instance_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }) {
        Ok(v) => Some(v),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(_) => {
            return Err(LauncherError::Generic {
                code: "ERR_LOCAL_STATE_FAILED".into(),
                message: "Failed to query approval grants.".into(),
            })
        }
    };

    let state: Option<String> = match row {
        Some((state, Some(ref expires_at))) => {
            // Treat expired session grants as "no grant" so the caller
            // receives the "approval required" error instead of a stale
            // allow. A cron or on-read expiry is sufficient; there is
            // no background expiry sweeper.
            match chrono::DateTime::parse_from_rfc3339(expires_at) {
                Ok(expiry) if expiry < chrono::Utc::now() => None,
                _ => Some(state),
            }
        }
        Some((state, None)) => Some(state),
        None => None,
    };

    match check_approval_grant(state.as_deref(), is_destructive) {
        ApprovalResult::Allowed => Ok(()),
        ApprovalResult::Denied => match state.as_deref() {
            Some("always_deny") => Err(LauncherError::McpDenied),
            None => Err(LauncherError::Generic {
                code: "ERR_MCP_DENIED".into(),
                message: format!(
                    "Approval required: grant '{tool_name}' for instance '{instance_id}' in Agora Settings",
                ),
            }),
            Some(other) => Err(LauncherError::Generic {
                code: "ERR_MCP_DENIED".into(),
                message: format!("Unknown approval state: {other}"),
            }),
        },
    }
}

// ---------------------------------------------------------------------------
// Approval grant write (parameterized upsert via Ctx)
// ---------------------------------------------------------------------------

/// Allowed grant states.
const ALLOWED_GRANT_STATES: [&str; 3] = ["always_allow", "always_deny", "session"];

/// Record or update an approval grant for a tool + instance pair.
///
/// Validates that `state` is one of the allowed values, sanitizes identifiers,
/// computes session expiry, and writes via a parameterized upsert.
pub fn set_approval_grant(
    ctx: &Ctx,
    tool_name: &str,
    instance_id: &str,
    state: &str,
) -> LauncherResult<()> {
    if !ALLOWED_GRANT_STATES.contains(&state) {
        return Err(LauncherError::Generic {
            code: "ERR_MCP_GRANT_INVALID_STATE".into(),
            message: format!(
                "Invalid approval state '{state}'. Must be one of: always_allow, always_deny, session"
            ),
        });
    }

    let tool_name = crate::paths::sanitize_id(tool_name);
    let instance_id = crate::paths::sanitize_id(instance_id);

    if tool_name.is_empty() || instance_id.is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_MCP_GRANT_INVALID_IDENTIFIER".into(),
            message: "Tool name and instance ID must not be empty after sanitization.".into(),
        });
    }

    let now = chrono::Utc::now().to_rfc3339();
    let expires_at = if state == "session" {
        Some((chrono::Utc::now() + chrono::Duration::hours(24)).to_rfc3339())
    } else {
        None
    };

    let db_path = ctx.paths.local_state_db();
    let conn =
        crate::db::local_state_connection(&db_path).map_err(|_| LauncherError::LocalStateFailed)?;

    conn.execute(
        "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(tool_name, instance_id) DO UPDATE SET
             state = excluded.state,
             granted_at = excluded.granted_at,
             expires_at = excluded.expires_at",
        rusqlite::params![tool_name, instance_id, state, now, expires_at],
    )
    .map_err(|_| LauncherError::LocalStateFailed)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_instance_manifest(
    ctx: &Ctx,
    instance_id: &str,
) -> LauncherResult<Option<InstanceManifest>> {
    let path = ctx.paths.instance_manifest(instance_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|_| LauncherError::Generic {
        code: "ERR_LOCAL_STATE_FAILED".into(),
        message: "Cannot read instance manifest.".into(),
    })?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|_| LauncherError::Generic {
            code: "ERR_LOCAL_STATE_FAILED".into(),
            message: "Cannot parse instance manifest.".into(),
        })
}

fn open_registry(ctx: &Ctx) -> LauncherResult<rusqlite::Connection> {
    let path = ctx.paths.registry_db();
    if !path.exists() {
        return Err(LauncherError::RegistryMissing);
    }
    crate::db::registry_connection(&path).map_err(|_| LauncherError::RegistryMissing)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // McpDispatcher struct
    // -----------------------------------------------------------------------

    #[test]
    fn dispatcher_struct_initialize() {
        let (ctx, _tmp) = with_ctx();
        let d = McpDispatcher::new(ctx);
        let result = d.initialize();
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "agora");
    }

    #[test]
    fn dispatcher_struct_list_tools() {
        let (ctx, _tmp) = with_ctx();
        let d = McpDispatcher::new(ctx);
        let result = d.list_tools();
        let tools = result["tools"].as_array().expect("tools should be array");
        assert!(!tools.is_empty());
    }

    #[test]
    fn dispatcher_struct_call_tool() {
        let (ctx, _tmp) = with_ctx();
        let d = McpDispatcher::new(ctx);
        let result = d.call_tool("list_instances", &serde_json::json!({}));
        let content = result["content"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("");
        assert!(
            content.contains("\"instances\""),
            "response should contain instances key"
        );
    }

    #[test]
    fn dispatcher_struct_list_resources() {
        let (ctx, _tmp) = with_ctx();
        let d = McpDispatcher::new(ctx);
        let result = d.list_resources();
        let resources = result["resources"]
            .as_array()
            .expect("resources should be array");
        assert!(!resources.is_empty());
    }

    #[test]
    fn dispatcher_struct_read_resource_unknown() {
        let (ctx, _tmp) = with_ctx();
        let d = McpDispatcher::new(ctx);
        let result = d.read_resource("nonexistent.md");
        assert!(result["error"].is_object());
        assert_eq!(result["error"]["code"], -32602);
    }

    #[test]
    fn dispatcher_struct_handle_method_unknown() {
        let (ctx, _tmp) = with_ctx();
        let d = McpDispatcher::new(ctx);
        let result = d.handle_method("bogus_method", None);
        assert!(result["error"].is_object());
        assert_eq!(result["error"]["code"], -32601);
    }

    #[test]
    fn dispatcher_struct_is_cloneable() {
        let (ctx, _tmp) = with_ctx();
        let d1 = McpDispatcher::new(ctx);
        let d2 = d1.clone();
        let r1 = d1.initialize();
        let r2 = d2.initialize();
        assert_eq!(r1, r2);
    }

    // -----------------------------------------------------------------------
    // Approval logic (pure, no DB)
    // -----------------------------------------------------------------------

    #[test]
    fn approval_always_allow() {
        assert_eq!(
            check_approval_grant(Some("always_allow"), true),
            ApprovalResult::Allowed
        );
    }

    #[test]
    fn approval_always_deny() {
        assert_eq!(
            check_approval_grant(Some("always_deny"), true),
            ApprovalResult::Denied
        );
    }

    #[test]
    fn approval_session() {
        assert_eq!(
            check_approval_grant(Some("session"), true),
            ApprovalResult::Allowed
        );
    }

    #[test]
    fn approval_no_grant_readonly() {
        assert_eq!(check_approval_grant(None, false), ApprovalResult::Allowed);
    }

    #[test]
    fn approval_no_grant_destructive() {
        assert_eq!(check_approval_grant(None, true), ApprovalResult::Denied);
    }

    #[test]
    fn approval_unknown_state() {
        assert_eq!(
            check_approval_grant(Some("unknown_value"), true),
            ApprovalResult::Denied
        );
    }

    // -----------------------------------------------------------------------
    // Dispatch response shapes
    // -----------------------------------------------------------------------

    static TEST_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    /// Create a test context with an initialized local_state.db.
    /// Each call gets a unique temp directory so parallel tests don't collide.
    fn with_ctx() -> (Ctx, std::path::PathBuf) {
        let n = TEST_CTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = std::env::temp_dir().join(format!("agora-mcp-test-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let ctx = Ctx::for_testing(tmp.clone());
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        (ctx, tmp)
    }

    #[test]
    fn initialize_returns_protocol_metadata() {
        let (ctx, _tmp) = with_ctx();
        let result = handle_mcp_method(&ctx, "initialize", None);
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "agora");
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["capabilities"]["resources"].is_object());
    }

    #[test]
    fn tools_list_returns_array() {
        let (ctx, _tmp) = with_ctx();
        let result = handle_mcp_method(&ctx, "tools/list", None);
        let tools = result["tools"].as_array().expect("tools should be array");
        assert!(!tools.is_empty(), "should have at least one tool");
        // Verify each tool has required fields
        for tool in tools {
            assert!(tool["name"].as_str().is_some(), "tool must have name");
            assert!(
                tool["description"].as_str().is_some(),
                "tool must have description"
            );
            assert!(
                tool["inputSchema"].is_object(),
                "tool must have inputSchema"
            );
        }
    }

    #[test]
    fn resources_list_has_system_context() {
        let (ctx, _tmp) = with_ctx();
        let result = handle_mcp_method(&ctx, "resources/list", None);
        let resources = result["resources"]
            .as_array()
            .expect("resources should be array");
        let uris: Vec<&str> = resources.iter().filter_map(|r| r["uri"].as_str()).collect();
        assert!(
            uris.contains(&"system_context.md"),
            "should include system_context.md: {uris:?}"
        );
    }

    #[test]
    fn resources_read_unknown_uri_returns_error() {
        let (ctx, _tmp) = with_ctx();
        let params = serde_json::json!({ "uri": "nonexistent.md" });
        let result = handle_mcp_method(&ctx, "resources/read", Some(&params));
        assert!(
            result["error"].is_object(),
            "unknown URI should produce error"
        );
        assert_eq!(result["error"]["code"], -32602);
    }

    #[test]
    fn unknown_method_returns_error() {
        let (ctx, _tmp) = with_ctx();
        let result = handle_mcp_method(&ctx, "bogus_method", None);
        assert!(result["error"].is_object(), "unknown method should error");
        assert_eq!(result["error"]["code"], -32601);
    }

    #[test]
    fn tool_call_unknown_tool_returns_error() {
        let (ctx, _tmp) = with_ctx();
        let params = serde_json::json!({
            "name": "nonexistent_tool",
            "arguments": {}
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));
        assert!(result["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn tool_call_list_instances_returns_valid_shape() {
        let (ctx, _tmp) = with_ctx();
        let params = serde_json::json!({
            "name": "list_instances",
            "arguments": {}
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));
        // Should not error even with empty instance list
        assert!(!result["isError"].as_bool().unwrap_or(true));
        let content = result["content"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("");
        assert!(
            content.contains("\"instances\""),
            "response should contain instances key"
        );
    }

    #[test]
    fn tool_call_list_instance_mods_empty() {
        let (ctx, _tmp) = with_ctx();
        let params = serde_json::json!({
            "name": "list_instance_mods",
            "arguments": { "instance_id": "nonexistent" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));
        let content = result["content"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("");
        assert!(
            content.contains("\"mods\""),
            "response should contain mods key even for nonexistent: {content}"
        );
    }

    #[test]
    fn tool_call_read_mod_manifest_unknown_id() {
        let (ctx, _tmp) = with_ctx();
        let params = serde_json::json!({
            "name": "read_mod_manifest",
            "arguments": { "mod_id": "nonexistent" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));
        // Without a registry DB, this should report an error gracefully.
        assert!(!result["content"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("")
            .is_empty());
    }

    #[test]
    fn tool_call_search_crash_signatures_returns_matches_or_empty() {
        let (ctx, _tmp) = with_ctx();
        let params = serde_json::json!({
            "name": "search_crash_signatures",
            "arguments": { "crash_text": "OutOfMemoryError: Java heap space" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));
        let content = result["content"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("");
        assert!(
            content.contains("\"matches\""),
            "response should contain matches: {content}"
        );
    }

    #[test]
    fn tool_call_search_knowledge_base_empty_registry() {
        let (ctx, _tmp) = with_ctx();
        let params = serde_json::json!({
            "name": "search_knowledge_base",
            "arguments": { "query": "sodium" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));
        // Without a registry DB, should handle gracefully (isError or empty results)
        let content = result["content"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("");
        assert!(!content.is_empty(), "should return a response");
    }

    // -----------------------------------------------------------------------
    // check_approval with real DB
    // -----------------------------------------------------------------------

    #[test]
    fn db_approval_readonly_always_passes() {
        let (ctx, tmp) = with_ctx();
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

        // Read-only tool — should pass without any grant row.
        assert!(check_approval(&ctx, "list_instances", "test", false).is_ok());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn db_approval_destructive_without_grant_denies() {
        let (ctx, tmp) = with_ctx();
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();

        let result = check_approval(&ctx, "disable_mod", "test", true);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), "ERR_MCP_DENIED");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn db_approval_destructive_with_always_allow() {
        let (ctx, tmp) = with_ctx();
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        conn.execute(
            "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at)
             VALUES ('disable_mod', 'test', 'always_allow', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(check_approval(&ctx, "disable_mod", "test", true).is_ok());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn db_approval_destructive_with_always_deny() {
        let (ctx, tmp) = with_ctx();
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        conn.execute(
            "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at)
             VALUES ('disable_mod', 'test', 'always_deny', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        let result = check_approval(&ctx, "disable_mod", "test", true);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "ERR_MCP_DENIED");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn db_approval_destructive_with_session_grant() {
        let (ctx, tmp) = with_ctx();
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        conn.execute(
            "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at)
             VALUES ('enable_mod', 'test', 'session', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        assert!(check_approval(&ctx, "enable_mod", "test", true).is_ok());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn db_approval_destructive_wrong_tool_denies() {
        let (ctx, tmp) = with_ctx();
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        conn.execute(
            "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at)
             VALUES ('disable_mod', 'test', 'always_allow', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        // Different tool — should deny
        let result = check_approval(&ctx, "enable_mod", "test", true);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "ERR_MCP_DENIED");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn db_approval_destructive_wrong_instance_denies() {
        let (ctx, tmp) = with_ctx();
        crate::db::init_local_state_db(&ctx.paths.local_state_db()).unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        conn.execute(
            "INSERT INTO mcp_approval_grants (tool_name, instance_id, state, granted_at)
             VALUES ('disable_mod', 'instance-a', 'always_allow', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);

        // Different instance — should deny
        let result = check_approval(&ctx, "disable_mod", "instance-b", true);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "ERR_MCP_DENIED");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // System context shape
    // -----------------------------------------------------------------------

    #[test]
    fn system_context_is_markdown() {
        let (ctx, tmp) = with_ctx();
        let md = build_system_context(&ctx);
        assert!(md.starts_with("# Agora System Context"));
        assert!(md.contains("## Instances"));
        assert!(md.contains("## Installed Mods"));
        assert!(md.contains("## Recent Crashes"));

        // With no instances, should show appropriate messages
        assert!(md.contains("No instances configured."));
        assert!(md.contains("No installed mods."));
        assert!(md.contains("No recent crash reports found."));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // Tool definitions completeness
    // -----------------------------------------------------------------------

    #[test]
    fn tool_definitions_includes_portable_tools() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"list_instances"));
        assert!(names.contains(&"list_instance_mods"));
        assert!(names.contains(&"read_mod_manifest"));
        assert!(names.contains(&"get_system_context"));
        assert!(names.contains(&"search_crash_signatures"));
        assert!(names.contains(&"search_knowledge_base"));
        assert!(names.contains(&"disable_mod"));
        assert!(names.contains(&"enable_mod"));
        assert!(names.contains(&"read_latest_crash"));
        assert!(names.contains(&"suggest_mod_incompatibility"));
    }

    // -----------------------------------------------------------------------
    // set_approval_grant — state validation and upsert shape
    // -----------------------------------------------------------------------

    #[test]
    fn set_approval_rejects_unknown_state() {
        let (ctx, tmp) = with_ctx();
        let result = set_approval_grant(&ctx, "test_tool", "test_instance", "bogus");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code(), "ERR_MCP_GRANT_INVALID_STATE");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_accepts_always_allow() {
        let (ctx, tmp) = with_ctx();
        let result = set_approval_grant(&ctx, "test_tool", "test_instance", "always_allow");
        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_accepts_always_deny() {
        let (ctx, tmp) = with_ctx();
        let result = set_approval_grant(&ctx, "test_tool", "test_instance", "always_deny");
        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_accepts_session() {
        let (ctx, tmp) = with_ctx();
        let result = set_approval_grant(&ctx, "test_tool", "test_instance", "session");
        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_rejects_empty_tool_name() {
        let (ctx, tmp) = with_ctx();
        let result = set_approval_grant(&ctx, "", "test_instance", "always_allow");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code(),
            "ERR_MCP_GRANT_INVALID_IDENTIFIER"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_rejects_empty_instance_id() {
        let (ctx, tmp) = with_ctx();
        let result = set_approval_grant(&ctx, "test_tool", "", "always_allow");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().code(),
            "ERR_MCP_GRANT_INVALID_IDENTIFIER"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_upsert_updates_existing_row() {
        let (ctx, tmp) = with_ctx();
        // Insert
        set_approval_grant(&ctx, "my_tool", "my_instance", "always_allow").unwrap();
        // Verify
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let state: String = conn
            .query_row(
                "SELECT state FROM mcp_approval_grants WHERE tool_name = 'my_tool' AND instance_id = 'my_instance'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "always_allow");

        // Upsert to a different state
        set_approval_grant(&ctx, "my_tool", "my_instance", "always_deny").unwrap();
        let state: String = conn
            .query_row(
                "SELECT state FROM mcp_approval_grants WHERE tool_name = 'my_tool' AND instance_id = 'my_instance'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "always_deny");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_session_expiry_is_set() {
        let (ctx, tmp) = with_ctx();
        set_approval_grant(&ctx, "session_tool", "session_inst", "session").unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let expires_at: Option<String> = conn
            .query_row(
                "SELECT expires_at FROM mcp_approval_grants WHERE tool_name = 'session_tool' AND instance_id = 'session_inst'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            expires_at.is_some(),
            "session grant should have an expires_at"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn set_approval_always_allow_expiry_is_null() {
        let (ctx, tmp) = with_ctx();
        set_approval_grant(&ctx, "persist_tool", "persist_inst", "always_allow").unwrap();
        let conn = crate::db::local_state_connection(&ctx.paths.local_state_db()).unwrap();
        let expires_at: Option<String> = conn
            .query_row(
                "SELECT expires_at FROM mcp_approval_grants WHERE tool_name = 'persist_tool' AND instance_id = 'persist_inst'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            expires_at.is_none(),
            "always_allow grant should have null expires_at"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // Dispatcher-level disable_mod / enable_mod / read_latest_crash
    // -----------------------------------------------------------------------

    /// Helper: create a minimal instance directory + manifest + mod file.
    fn create_test_instance(ctx: &Ctx, instance_id: &str, mod_filenames: &[&str]) {
        let instance_dir = ctx.paths.instance_dir(instance_id).unwrap();
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();

        for fname in mod_filenames {
            std::fs::write(instance_dir.join("mods").join(fname), b"fake mod").unwrap();
        }

        let manifest = crate::models::InstanceManifest {
            instance_id: instance_id.to_string(),
            name: instance_id.to_string(),
            created_from_pack: None,
            minecraft_version: "1.20".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: mod_filenames
                .iter()
                .map(|fname| crate::models::InstalledMod {
                    filename: fname.to_string(),
                    registry_id: None,
                    modrinth_id: None,
                    source: "local".into(),
                    source_url: None,
                    version: Some("1.0.0".into()),
                    sha256: "abc".into(),
                    installed_at: "2024-01-01T00:00:00Z".into(),
                    java_packages: vec![],
                    mod_jar_id: None,
                    provided_mod_ids: vec![],
                    enabled: true,
                    content_type: "mod".into(),
                    depends_on: vec![],
                    optional_deps: vec![],
                    incompatible_deps: vec![],
                })
                .collect(),
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::Value::Null,
        };
        let manifest_path = ctx.paths.instance_manifest(instance_id).unwrap();
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn tool_call_disable_mod_denied_without_approval() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-a", &["some-mod.jar"]);

        let params = serde_json::json!({
            "name": "disable_mod",
            "arguments": { "instance_id": "test-a", "filename": "some-mod.jar" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(result["isError"].as_bool().unwrap_or(false));
        let content = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            content.contains("Approval denied"),
            "should contain denial message: {content}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_enable_mod_denied_without_approval() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-a", &["some-mod.jar"]);

        let params = serde_json::json!({
            "name": "enable_mod",
            "arguments": { "instance_id": "test-a", "filename": "some-mod.jar" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(result["isError"].as_bool().unwrap_or(false));
        let content = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            content.contains("Approval denied"),
            "should contain denial message: {content}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_disable_mod_approved_succeeds() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-b", &["target.jar"]);

        // Grant approval
        set_approval_grant(&ctx, "disable_mod", "test-b", "always_allow").unwrap();

        let params = serde_json::json!({
            "name": "disable_mod",
            "arguments": { "instance_id": "test-b", "filename": "target.jar" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "should succeed: {result}"
        );
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("disabled"), "should mention disabled: {text}");

        // Verify file was renamed
        let disabled_path = ctx
            .paths
            .instance_dir("test-b")
            .unwrap()
            .join("mods")
            .join("target.jar.disabled");
        assert!(disabled_path.exists(), "disabled file should exist");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_enable_mod_approved_succeeds() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-c", &["re-enable.jar"]);

        // First disable the mod via CrashService directly
        let svc = crate::crash_service::CrashService::new(ctx.clone());
        svc.disable_mod("test-c", "re-enable.jar").unwrap();

        // Grant approval for enable
        set_approval_grant(&ctx, "enable_mod", "test-c", "always_allow").unwrap();

        let params = serde_json::json!({
            "name": "enable_mod",
            "arguments": { "instance_id": "test-c", "filename": "re-enable.jar" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "should succeed: {result}"
        );
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("re-enabled"),
            "should mention re-enabled: {text}"
        );

        // Verify file was renamed back
        let active_path = ctx
            .paths
            .instance_dir("test-c")
            .unwrap()
            .join("mods")
            .join("re-enable.jar");
        assert!(active_path.exists(), "re-enabled file should exist");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_read_latest_crash_no_reports() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-d", &[]);

        let params = serde_json::json!({
            "name": "read_latest_crash",
            "arguments": { "instance_id": "test-d" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        // Should NOT be an error — "no reports" is a normal state
        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "no-reports should not error: {result}"
        );
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("No crash reports found"),
            "should mention no reports: {text}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_read_latest_crash_with_content() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-e", &[]);

        // Create a crash report file in the crash-reports directory
        let crash_dir = ctx
            .paths
            .instance_dir("test-e")
            .unwrap()
            .join("crash-reports");
        std::fs::create_dir_all(&crash_dir).unwrap();

        // Write a crash log with 5 lines
        let crash_content = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        std::fs::write(
            crash_dir.join("crash-2024-01-01_12.00.00.txt"),
            crash_content,
        )
        .unwrap();

        let params = serde_json::json!({
            "name": "read_latest_crash",
            "arguments": { "instance_id": "test-e" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "should not error: {result}"
        );
        assert_eq!(result["filename"], "crash-2024-01-01_12.00.00.txt");
        assert_eq!(result["total_lines"], 5);

        let text = result["content"][0]["text"].as_str().unwrap_or("");
        // Should contain all 5 lines since they're under 200
        assert!(
            text.contains("line 3"),
            "tail should include line 3: {text}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // -----------------------------------------------------------------------
    // suggest_mod_incompatibility tests
    // -----------------------------------------------------------------------

    #[test]
    fn tool_call_suggest_mod_incompatibility_empty_crash_text() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-ic1", &["some-mod.jar"]);

        let params = serde_json::json!({
            "name": "suggest_mod_incompatibility",
            "arguments": { "instance_id": "test-ic1", "crash_text": "" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        // Empty crash text: no fingerprint → suspects with score 0
        assert!(!result["isError"].as_bool().unwrap_or(true));
        let content = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            content.contains("\"suspects\""),
            "should contain suspects: {content}"
        );
        assert!(
            content.contains("\"total_score\":0.0"),
            "expected 0.0 scores for empty crash text: {content}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_suggest_mod_incompatibility_nonexistent_instance() {
        let (ctx, tmp) = with_ctx();

        let params = serde_json::json!({
            "name": "suggest_mod_incompatibility",
            "arguments": { "instance_id": "does-not-exist", "crash_text": "java.lang.Exception at foo.Bar.baz(Foo.java:42)" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(
            result["isError"].as_bool().unwrap_or(false),
            "should error: {result}"
        );
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("not found"),
            "should mention not found: {text}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_suggest_mod_incompatibility_matching_suspects() {
        let (ctx, tmp) = with_ctx();
        // Create instance with a mod whose java_packages match the crash stack.
        create_test_instance_with_packages(
            &ctx,
            "test-ic2",
            &[("suspect-mod.jar", vec!["com.example".into()])],
        );

        let crash_text = "\
Exception in thread \"main\" java.lang.RuntimeException: Test
    at com.example.mod.Core.init(Core.java:10)
    at net.minecraft.client.Minecraft.tick(Minecraft.java:100)";

        let params = serde_json::json!({
            "name": "suggest_mod_incompatibility",
            "arguments": { "instance_id": "test-ic2", "crash_text": crash_text }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "should not error: {result}"
        );
        let content = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            content.contains("\"suspects\""),
            "should contain suspects: {content}"
        );
        assert!(
            content.contains("suspect-mod"),
            "should contain the mod name: {content}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Helper: create a minimal instance with mods that have custom java_packages.
    fn create_test_instance_with_packages(
        ctx: &Ctx,
        instance_id: &str,
        mods: &[(&str, Vec<String>)],
    ) {
        let instance_dir = ctx.paths.instance_dir(instance_id).unwrap();
        std::fs::create_dir_all(instance_dir.join("mods")).unwrap();

        for (fname, _) in mods {
            std::fs::write(instance_dir.join("mods").join(fname), b"fake mod").unwrap();
        }

        let manifest = crate::models::InstanceManifest {
            instance_id: instance_id.to_string(),
            name: instance_id.to_string(),
            created_from_pack: None,
            minecraft_version: "1.20".into(),
            loader: "fabric".into(),
            loader_version: "0.15.0".into(),
            is_locked: false,
            mods: mods
                .iter()
                .map(|(fname, packages)| crate::models::InstalledMod {
                    filename: fname.to_string(),
                    registry_id: Some(fname.trim_end_matches(".jar").to_string()),
                    modrinth_id: None,
                    source: "local".into(),
                    source_url: None,
                    version: Some("1.0.0".into()),
                    sha256: "abc".into(),
                    installed_at: "2024-01-01T00:00:00Z".into(),
                    java_packages: packages.clone(),
                    mod_jar_id: None,
                    provided_mod_ids: vec![],
                    enabled: true,
                    content_type: "mod".into(),
                    depends_on: vec![],
                    optional_deps: vec![],
                    incompatible_deps: vec![],
                })
                .collect(),
            resourcepacks: vec![],
            shaders: vec![],
            datapacks: vec![],
            worlds: vec![],
            user_preferences: serde_json::Value::Null,
        };
        let manifest_path = ctx.paths.instance_manifest(instance_id).unwrap();
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn tool_call_suggest_mod_incompatibility_no_suspects() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-ic3", &["unrelated-mod.jar"]);

        let crash_text = "\
Exception in thread \"main\" java.lang.RuntimeException: Test
    at com.unknown.mod.Core.init(Core.java:10)";

        let params = serde_json::json!({
            "name": "suggest_mod_incompatibility",
            "arguments": { "instance_id": "test-ic3", "crash_text": crash_text }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "should not error: {result}"
        );
        let content = result["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            content.contains("\"total_score\":0.0"),
            "expected 0.0 scores for unrelated mod: {content}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_suggest_mod_incompatibility_tool_definition_present() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(
            names.contains(&"suggest_mod_incompatibility"),
            "should be in tool definitions: {names:?}"
        );
    }

    #[test]
    fn tool_call_suggest_mod_incompatibility_readonly_no_approval_required() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-ic4", &["test.jar"]);

        // Call without any approval grant — should succeed because the tool is read-only.
        let params = serde_json::json!({
            "name": "suggest_mod_incompatibility",
            "arguments": { "instance_id": "test-ic4", "crash_text": "java.lang.Exception at x.y.z(X.java:1)" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(
            !result["isError"].as_bool().unwrap_or(true),
            "read-only tool should not require approval: {result}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_suggest_mod_incompatibility_dispatcher_struct_method() {
        // Verify the dispatcher struct method works for this tool
        let (ctx, tmp) = with_ctx();
        let d = McpDispatcher::new(ctx);
        let result = d.call_tool(
            "suggest_mod_incompatibility",
            &serde_json::json!({ "instance_id": "nonexistent", "crash_text": "test" }),
        );
        let content = result["content"]
            .as_array()
            .and_then(|c| c.first())
            .and_then(|c| c["text"].as_str())
            .unwrap_or("");
        // Unknown instance should produce an error
        assert!(
            content.contains("not found"),
            "should contain not found: {content}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn tool_call_read_latest_crash_tail_only_200_lines() {
        let (ctx, tmp) = with_ctx();
        create_test_instance(&ctx, "test-f", &[]);

        let crash_dir = ctx
            .paths
            .instance_dir("test-f")
            .unwrap()
            .join("crash-reports");
        std::fs::create_dir_all(&crash_dir).unwrap();

        // Write a crash log with 250 lines (should only return last 200)
        let lines_250: Vec<String> = (1..=250).map(|i| format!("line {}", i)).collect();
        let crash_content = lines_250.join("\n") + "\n";
        std::fs::write(crash_dir.join("crash-large.txt"), &crash_content).unwrap();

        let params = serde_json::json!({
            "name": "read_latest_crash",
            "arguments": { "instance_id": "test-f" }
        });
        let result = handle_mcp_method(&ctx, "tools/call", Some(&params));

        assert!(!result["isError"].as_bool().unwrap_or(true));
        assert_eq!(result["filename"], "crash-large.txt");
        assert_eq!(result["total_lines"], 250);

        let text = result["content"][0]["text"].as_str().unwrap_or("");
        let returned_lines: Vec<&str> = text.lines().collect();
        assert_eq!(returned_lines.len(), 200, "should return 200 lines");
        assert!(
            text.contains("line 51"),
            "tail should start around line 51: {text}"
        );
        assert!(text.contains("line 250"), "tail should include last line");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
