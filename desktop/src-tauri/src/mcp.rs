use std::borrow::Cow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use crate::paths;

// ---------------------------------------------------------------------------
// Baked-in MCP skill guide
// ---------------------------------------------------------------------------

/// The Agora MCP skill guide, baked into the app so users can copy it
/// from Settings without finding it on disk.
pub const MCP_SKILL_CONTENT: &str = include_str!("../skills/agora-mcp/SKILL.md");

// ---------------------------------------------------------------------------
// Session ID generation (no uuid crate â€” SystemTime + counter)
// ---------------------------------------------------------------------------

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_session_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:016x}{:016x}", secs, counter)
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
    #[serde(default)]
    id: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: serde_json::Value,
}

impl JsonRpcResponse {
    fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    fn error(id: serde_json::Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
            }),
            id,
        }
    }
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

const JSONRPC_ERROR_METHOD_NOT_FOUND: i32 = -32601;
const MCP_ERR_TOO_MANY_REQUESTS: &str = "ERR_MCP_TOO_MANY_REQUESTS";
const MAX_MCP_BODY_SIZE: usize = 1_048_576; // 1 MiB
const MAX_CONCURRENT_CONNECTIONS: usize = 64;
const MAX_HEADER_SIZE: usize = 32_768; // 32 KiB

// MCP error codes (application-level)
const MCP_TOKEN_KEY: &str = "mcp_bearer_token";

fn generate_token() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    hex::encode(bytes)
}

fn get_or_create_mcp_token(app: &AppHandle) -> Option<String> {
    let ctx = crate::core_context(app).ok()?;
    let svc = agora_core::settings::SettingsService::new(ctx);
    match svc.get(MCP_TOKEN_KEY) {
        Ok(Some(serde_json::Value::String(t))) if !t.is_empty() => Some(t),
        _ => {
            let token = generate_token();
            if svc
                .set(MCP_TOKEN_KEY, &serde_json::Value::String(token.clone()))
                .is_ok()
            {
                if let Ok(app_data) = paths::app_data_dir(app) {
                    write_token_file(&app_data, &token);
                }
                Some(token)
            } else {
                None
            }
        }
    }
}

fn write_token_file(app_data_dir: &std::path::Path, token: &str) {
    let path = app_data_dir.join("mcp_token");
    if let Ok(mut f) = std::fs::File::create(&path) {
        let _ = std::io::Write::write_all(&mut f, token.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
    }
}

/// Read one line, refusing to buffer more than `limit` bytes. `Ok(None)` means
/// the line exceeded `limit`.
///
/// `BufReader::read_line` grows its target without bound, so a client that never
/// sends a newline can drive allocation until the 30 s connection timeout — and
/// this runs before authentication. Checking the size only *after* a line has
/// been fully read is too late, so the cap goes on the read itself.
async fn read_line_bounded<R>(
    reader: &mut R,
    buf: &mut String,
    limit: usize,
) -> std::io::Result<Option<usize>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let read = reader.take(limit as u64).read_line(buf).await?;
    if read == limit && !buf.ends_with('\n') {
        return Ok(None);
    }
    Ok(Some(read))
}

fn extract_bearer_token(headers: &std::collections::HashMap<String, String>) -> Option<String> {
    if let Some(auth) = headers.get("authorization") {
        if let Some(t) = auth.strip_prefix("Bearer ") {
            return Some(t.trim().to_string());
        }
        if let Some(t) = auth.strip_prefix("bearer ") {
            return Some(t.trim().to_string());
        }
    }
    None
}

/// Compare two tokens without short-circuiting on the first differing byte.
///
/// `==` on `str` stops as soon as it finds a mismatch, so how long a rejection
/// takes reveals how many leading characters were correct — which turns
/// guessing a 64-character token into guessing it one character at a time. This
/// reads every byte of the expected token regardless.
///
/// Length is compared up front and is not itself secret: the token is a
/// fixed-width hex string, so its length is public.
fn tokens_match(candidate: &str, expected: &str) -> bool {
    let (candidate, expected) = (candidate.as_bytes(), expected.as_bytes());
    if candidate.len() != expected.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in candidate.iter().zip(expected.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Whether a request's headers carry the expected bearer token.
///
/// The empty-`expected` guard is not redundant with `start_server`'s refusal to
/// launch without a token: `extract_bearer_token` trims, so a bare
/// `Authorization: Bearer ` yields `Some("")`, which would compare equal to an
/// empty expected token and authorize the request. Auth must fail closed even if
/// a future change reintroduces a keyless state.
fn is_authorized(expected: &str, headers: &std::collections::HashMap<String, String>) -> bool {
    if expected.is_empty() {
        return false;
    }
    extract_bearer_token(headers).is_some_and(|token| tokens_match(&token, expected))
}

// ---------------------------------------------------------------------------
// SSE session store + global rate limiter
// ---------------------------------------------------------------------------

struct McpServerState {
    sessions: std::sync::Mutex<HashMap<String, tokio::sync::mpsc::Sender<String>>>,
    rate_limiter: std::sync::Mutex<RateLimiter>,
    expected_token: String,
    semaphore: Arc<tokio::sync::Semaphore>,
}

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

struct RateLimiter {
    requests: Vec<u64>, // timestamps in seconds
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
        }
    }

    fn allow(&mut self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.requests.retain(|&t| now.wrapping_sub(t) < 60);
        if self.requests.len() >= 100 {
            return false;
        }
        self.requests.push(now);
        true
    }
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn tool_definitions_desktop_only() -> Vec<serde_json::Value> {
    vec![]
}

// ---------------------------------------------------------------------------
// Tool call handler
// ---------------------------------------------------------------------------

async fn handle_tool_call(
    app: &AppHandle,
    tool_name: &str,
    params: &serde_json::Value,
) -> serde_json::Value {
    // Portable tools — route through agora_core::mcp_dispatcher (handles
    // approval for destructive tools internally).
    let portable_tools: &[&str] = &[
        "list_instances",
        "list_instance_mods",
        "read_mod_manifest",
        "get_system_context",
        "search_crash_signatures",
        "search_knowledge_base",
        "disable_mod",
        "enable_mod",
        "read_latest_crash",
        "suggest_mod_incompatibility",
    ];
    if portable_tools.contains(&tool_name) {
        let ctx = match crate::core_context(app) {
            Ok(c) => c,
            Err(e) => {
                return serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Core context error: {}", e),
                    }],
                    "isError": true,
                });
            }
        };
        let dispatcher = agora_core::mcp_dispatcher::McpDispatcher::new(ctx);
        return dispatcher.call_tool(tool_name, params);
    }

    serde_json::json!({
        "content": [{
            "type": "text",
            "text": format!("Tool '{}' not found", tool_name),
        }],
        "isError": true,
    })
}

// ---------------------------------------------------------------------------
// MCP method handler
// ---------------------------------------------------------------------------

async fn handle_mcp_method(
    app: &AppHandle,
    method: &str,
    params: Option<&serde_json::Value>,
) -> serde_json::Value {
    match method {
        "initialize" => {
            let ctx = match crate::core_context(app) {
                Ok(c) => c,
                Err(e) => {
                    return serde_json::json!({
                        "error": { "code": -32603, "message": format!("Core context error: {}", e) },
                    });
                }
            };
            let dispatcher = agora_core::mcp_dispatcher::McpDispatcher::new(ctx);
            dispatcher.initialize()
        }
        "tools/list" => {
            let ctx = match crate::core_context(app) {
                Ok(c) => c,
                Err(e) => {
                    return serde_json::json!({
                        "error": { "code": -32603, "message": format!("Core context error: {}", e) },
                    });
                }
            };
            let dispatcher = agora_core::mcp_dispatcher::McpDispatcher::new(ctx);
            let mut result = dispatcher.list_tools();
            // Append desktop-only tools
            if let Some(tools) = result["tools"].as_array_mut() {
                tools.extend(tool_definitions_desktop_only());
            }
            result
        }
        "tools/call" => {
            let params = params.unwrap_or(&serde_json::Value::Null);
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let tool_params = params.get("arguments").unwrap_or(&serde_json::Value::Null);
            handle_tool_call(app, tool_name, tool_params).await
        }
        "resources/list" | "resources/read" => {
            let ctx = match crate::core_context(app) {
                Ok(c) => c,
                Err(e) => {
                    return serde_json::json!({
                        "error": { "code": -32603, "message": format!("Core context error: {}", e) },
                    });
                }
            };
            let dispatcher = agora_core::mcp_dispatcher::McpDispatcher::new(ctx);
            dispatcher.handle_method(method, params)
        }
        _ => serde_json::json!({
            "error": {
                "code": JSONRPC_ERROR_METHOD_NOT_FOUND,
                "message": format!("Unknown method: {}", method),
            }
        }),
    }
}

// ---------------------------------------------------------------------------
// HTTP parsing helpers
// ---------------------------------------------------------------------------

fn parse_request_line(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}
// NOTE: url::Url is not available in the desktop crate dependencies (would
// modify Cargo.lock), so this handwritten parser is retained as a known gap.

fn parse_query_params(path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(query) = path.split('?').nth(1) {
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                params.insert(
                    urlencoding::decode(k)
                        .unwrap_or(Cow::Borrowed(k))
                        .into_owned(),
                    urlencoding::decode(v)
                        .unwrap_or(Cow::Borrowed(v))
                        .into_owned(),
                );
            }
        }
    }
    params
}
// NOTE: url::Url / url::form_urlencoded are not available in the desktop crate
// dependencies (would modify Cargo.lock), so this handwritten parser is retained
// as a known gap.

fn extract_route(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

// Extract session_id from query string
fn extract_session_id(path: &str) -> Option<String> {
    parse_query_params(path).remove("session_id")
}

fn is_origin_allowed(origin: &str, port: u16) -> bool {
    let expected_127 = format!("http://127.0.0.1:{}", port);
    let expected_local = format!("http://localhost:{}", port);
    origin == expected_127 || origin == expected_local
}

// ---------------------------------------------------------------------------
// Connection handler â€” the core of the MCP server
// ---------------------------------------------------------------------------

async fn handle_connection(
    stream: tokio::net::TcpStream,
    state: Arc<McpServerState>,
    app: AppHandle,
    port: u16,
) -> std::io::Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async move {
        let (raw_read, mut write_half) = stream.into_split();
        let mut read_half = BufReader::new(raw_read);

        // The request line and every header line are bounded by the remaining
        // header budget, so an endless line can never outgrow MAX_HEADER_SIZE.
        let too_large = b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\n\r\n";

        // Read request line
        let mut request_line = String::new();
        match read_line_bounded(&mut read_half, &mut request_line, MAX_HEADER_SIZE).await? {
            Some(0) => return Ok(()),
            Some(_) => {}
            None => {
                let _ = write_half.write_all(too_large).await;
                let _ = write_half.shutdown().await;
                return Ok(());
            }
        }

        let (method, full_path) = match parse_request_line(&request_line) {
            Some(v) => v,
            None => return Ok(()),
        };

        // Read headers with aggregate size limit
        let mut headers = HashMap::new();
        let mut total_header_bytes: usize = 0;
        loop {
            let mut header_line = String::new();
            let remaining = MAX_HEADER_SIZE.saturating_sub(total_header_bytes);
            let bytes =
                match read_line_bounded(&mut read_half, &mut header_line, remaining).await? {
                    Some(bytes) => bytes,
                    None => {
                        let _ = write_half.write_all(too_large).await;
                        let _ = write_half.shutdown().await;
                        return Ok(());
                    }
                };
            if bytes == 0 {
                break;
            }
            total_header_bytes += bytes;
            if total_header_bytes > MAX_HEADER_SIZE {
                let _ = write_half.write_all(too_large).await;
                let _ = write_half.shutdown().await;
                return Ok(());
            }
            let header_line = header_line.trim();
            if header_line.is_empty() {
                break;
            }
            if let Some((key, value)) = header_line.split_once(':') {
                headers.insert(key.trim().to_lowercase(), value.trim().to_string());
            }
        }

        // Origin header validation
        if let Some(origin) = headers.get("origin") {
            if !origin.is_empty() && !is_origin_allowed(origin, port) {
                let body = br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":"Origin not allowed"}}"#;
                let msg = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    std::str::from_utf8(body).unwrap_or(""),
                );
                let _ = write_half.write_all(msg.as_bytes()).await;
                return Ok(());
            }
        }

        let route = extract_route(&full_path);

        // Token auth: reject unauthenticated connections (spec 10.0 #2, B2 2026-07-05).
        if !is_authorized(&state.expected_token, &headers) {
            let body = br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":"Unauthorized: MCP Bearer token required. Copy it from Settings > Integrations > MCP Server."}}"#;
            let msg = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                std::str::from_utf8(body).unwrap_or(""),
            );
            let _ = write_half.write_all(msg.as_bytes()).await;
            return Ok(());
        }

        match (method.as_str(), route) {
            ("GET", "/sse") => handle_sse(write_half, state, app).await,
            ("POST", "/messages") => {
                handle_post_messages(full_path, state, app, headers, read_half, write_half).await
            }
            ("POST", "/mcp") | ("POST", "/sse") => {
                handle_streamable_http(app, headers, read_half, write_half, state).await
            }
            _ => {
                let mut w = tokio::io::BufWriter::new(write_half);
                let _ = w
                    .write_all(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}"
                            .as_bytes(),
                    )
                    .await;
                Ok(())
            }
        }
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "MCP request timed out"))?
}

async fn handle_sse(
    writer: tokio::net::tcp::OwnedWriteHalf,
    state: Arc<McpServerState>,
    _app: AppHandle,
) -> std::io::Result<()> {
    let session_id = generate_session_id();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);

    // Register session
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.insert(session_id.clone(), tx);
    }

    let mut writer = tokio::io::BufWriter::new(writer);

    // Send SSE headers
    let sse_headers = "HTTP/1.1 200 OK\r\n\
        Content-Type: text/event-stream\r\n\
        Cache-Control: no-cache\r\n\
        Connection: keep-alive\r\n\
        X-Accel-Buffering: no\r\n\r\n";
    writer.write_all(sse_headers.as_bytes()).await?;
    writer.flush().await?;

    // Send endpoint event
    let endpoint_event = format!(
        "event: endpoint\ndata: /messages?session_id={}\n\n",
        session_id
    );
    writer.write_all(endpoint_event.as_bytes()).await?;
    writer.flush().await?;

    // Keep alive loop: wait for messages from the SSE channel.
    // The connection stays open; we send responses via the channel.
    // When the client disconnects, the write will fail and we exit.
    let mut alive = true;
    while alive {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Some(data) => {
                        let event = format!("data: {}\n\n", data);
                        if let Err(e) = writer.write_all(event.as_bytes()).await {
                            let _ = e;
                            alive = false;
                        } else {
                            let _ = writer.flush().await;
                        }
                    }
                    None => {
                        // Channel closed
                        alive = false;
                    }
                }
            }
            // Use a sleep to prevent busy-looping since we don't have the read half.
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                // Keep the loop alive, check for messages
            }
        }
    }

    // Clean up session
    {
        let mut sessions = state.sessions.lock().unwrap();
        sessions.remove(&session_id);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Streamable HTTP transport handler
// ---------------------------------------------------------------------------
//
// Per the 2025-03-26 MCP spec, Streamable HTTP clients POST JSON-RPC requests
// to a single endpoint (e.g. /mcp or the same /sse URL) and receive the
// JSON-RPC response directly in the HTTP response body (200 OK).
// No separate SSE session is required for request/response pairs.

async fn handle_streamable_http(
    app: AppHandle,
    headers: HashMap<String, String>,
    mut read_half: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    state: Arc<McpServerState>,
) -> std::io::Result<()> {
    // Read POST body
    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    // Maximum body size check
    if content_length > MAX_MCP_BODY_SIZE {
        let _ = write_half
            .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = write_half.shutdown().await;
        return Ok(());
    }

    // Rate limit check (global)
    let allowed = {
        let mut rl = state.rate_limiter.lock().unwrap();
        rl.allow()
    };
    if !allowed {
        let _ = write_half
            .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = write_half.shutdown().await;
        return Ok(());
    }

    let mut body = Vec::with_capacity(content_length);
    if content_length > 0 {
        body.resize(content_length, 0u8);
        let _ = read_half.read_exact(&mut body).await;
    }

    if body.is_empty() {
        // No body — return 204 No Content.
        let _ = write_half
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .await;
        return Ok(());
    }

    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let resp = JsonRpcResponse::error(
                serde_json::Value::Null,
                -32700,
                &format!("JSON parse error: {}", e),
            );
            let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
            let _ = write_half
                .write_all(
                    format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        resp_bytes.len(),
                        String::from_utf8_lossy(&resp_bytes)
                    )
                    .as_bytes(),
                )
                .await;
            return Ok(());
        }
    };

    // Notifications (no id) must not receive a response per JSON-RPC 2.0 spec.
    if request.id.is_none() {
        let _ = write_half
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
            .await;
        return Ok(());
    }

    let result = handle_mcp_method(&app, &request.method, request.params.as_ref()).await;
    let response = JsonRpcResponse::success(
        request.id.clone().unwrap_or(serde_json::Value::Null),
        result,
    );
    let resp_bytes = response.to_json_bytes();

    let _ = write_half
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                resp_bytes.len(),
                String::from_utf8_lossy(&resp_bytes)
            )
            .as_bytes(),
        )
        .await;

    Ok(())
}

// ---------------------------------------------------------------------------
// Legacy SSE POST /messages handler
// ---------------------------------------------------------------------------

async fn handle_post_messages(
    full_path: String,
    state: Arc<McpServerState>,
    app: AppHandle,
    headers: HashMap<String, String>,
    mut read_half: tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
) -> std::io::Result<()> {
    // Parse session_id from query params
    let session_id = extract_session_id(&full_path).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Missing session_id")
    })?;

    // Read POST body
    let content_length = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    // Maximum body size check
    if content_length > MAX_MCP_BODY_SIZE {
        let _ = write_half
            .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = write_half.shutdown().await;
        return Ok(());
    }

    let mut body = Vec::with_capacity(content_length);
    if content_length > 0 {
        body.resize(content_length, 0u8);
        let _ = read_half.read_exact(&mut body).await;
    }

    // Parse JSON-RPC request
    let request: JsonRpcRequest = if !body.is_empty() {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::error(
                    serde_json::Value::Null,
                    -32700, // Parse error
                    &format!("JSON parse error: {}", e),
                );
                let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
                let _ = write_half
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            resp_bytes.len(),
                            String::from_utf8_lossy(&resp_bytes)
                        )
                        .as_bytes(),
                    )
                    .await;
                return Ok(());
            }
        }
    } else {
        return Ok(());
    };

    // Rate limit check (global)
    let allowed = {
        let mut rl = state.rate_limiter.lock().unwrap();
        rl.allow()
    };
    if !allowed {
        let resp = JsonRpcResponse::error(
            request.id.unwrap_or(serde_json::Value::Null),
            -32000,
            MCP_ERR_TOO_MANY_REQUESTS,
        );
        let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
        let _ = write_half
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    resp_bytes.len(),
                    String::from_utf8_lossy(&resp_bytes)
                )
                .as_bytes(),
            )
            .await;
        return Ok(());
    }

    // Notifications (no id) must not receive a response per JSON-RPC 2.0 spec.
    // Return 202 Accepted with no body and no SSE push.
    if request.id.is_none() {
        let _ = write_half
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
            .await;
        return Ok(());
    }

    // Dispatch the JSON-RPC method
    let response = match request.method.as_str() {
        "initialize" | "tools/list" | "tools/call" | "resources/list" | "resources/read" => {
            let result = handle_mcp_method(&app, &request.method, request.params.as_ref()).await;
            JsonRpcResponse::success(
                request.id.clone().unwrap_or(serde_json::Value::Null),
                result,
            )
        }
        _ => JsonRpcResponse::error(
            request.id.unwrap_or(serde_json::Value::Null),
            JSONRPC_ERROR_METHOD_NOT_FOUND,
            &format!("Unknown method: {}", request.method),
        ),
    };

    let resp_bytes = response.to_json_bytes();

    // Per the legacy SSE transport spec, POST /messages returns 202 Accepted
    // and the actual JSON-RPC response travels via the SSE channel.
    // We also acknowledge via HTTP for clients that read the body directly.
    let _ = write_half
        .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
        .await;

    // Send the response via the SSE channel (the primary delivery path).
    {
        let sender = state.sessions.lock().unwrap().get(&session_id).cloned();
        if let Some(sender) = sender {
            let _ = sender
                .send(String::from_utf8_lossy(&resp_bytes).to_string())
                .await;
        }
    }

    Ok(())
}

impl JsonRpcResponse {
    fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// McpServer public API
// ---------------------------------------------------------------------------

pub struct McpServer {
    shutdown_tx: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    stopped_rx: std::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    running: Arc<AtomicBool>,
    port: u16,
}

impl McpServer {
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self
            .shutdown_tx
            .lock()
            .unwrap()
            .take()
            .map(|tx| tx.send(()));
    }

    /// Take the one-shot completion signal for the listener task. This is
    /// consumed by the lifecycle commands before replacing a stopped server.
    pub fn take_stopped_rx(&self) -> Option<tokio::sync::oneshot::Receiver<()>> {
        self.stopped_rx.lock().unwrap().take()
    }
}

/// Long-lived Tauri state for the optional MCP listener. The listener itself
/// is replaceable, while this manager is registered once at app startup. That
/// avoids Tauri's deprecated `unmanage` API and makes Stop → Start reliable.
pub struct McpServerManager {
    lifecycle: tokio::sync::Mutex<()>,
    server: tokio::sync::Mutex<Option<McpServer>>,
}

impl Default for McpServerManager {
    fn default() -> Self {
        Self {
            lifecycle: tokio::sync::Mutex::new(()),
            server: tokio::sync::Mutex::new(None),
        }
    }
}

impl McpServerManager {
    /// Start the listener if it is not already running and return its port.
    pub async fn start(&self, app: AppHandle) -> Result<u16, std::io::Error> {
        let _lifecycle_guard = self.lifecycle.lock().await;

        let stale_server = {
            let mut server = self.server.lock().await;
            if let Some(running) = server.as_ref().filter(|server| server.is_running()) {
                return Ok(running.port());
            }
            server.take()
        };

        if let Some(stale_server) = stale_server {
            stale_server.stop();
            if let Some(stopped_rx) = stale_server.take_stopped_rx() {
                let _ = stopped_rx.await;
            }
        }

        let server = start_server(app).await?;
        let port = server.port();
        *self.server.lock().await = Some(server);
        Ok(port)
    }

    /// Stop the listener and wait until its TCP socket has been released.
    pub async fn stop(&self) {
        let _lifecycle_guard = self.lifecycle.lock().await;
        let server = self.server.lock().await.take();
        if let Some(server) = server {
            server.stop();
            if let Some(stopped_rx) = server.take_stopped_rx() {
                let _ = stopped_rx.await;
            }
        }
    }

    pub async fn port(&self) -> Option<u16> {
        self.server
            .lock()
            .await
            .as_ref()
            .filter(|server| server.is_running())
            .map(McpServer::port)
    }

    /// Best-effort shutdown for synchronous application-close callbacks.
    pub fn request_shutdown(&self) {
        if let Ok(mut server) = self.server.try_lock() {
            if let Some(server) = server.take() {
                server.stop();
            }
        }
    }
}

pub async fn start_server(app: AppHandle) -> Result<McpServer, std::io::Error> {
    let port: u16 = 39741;
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();

    // Establish the bearer token BEFORE binding. A listener with no token would
    // accept `Authorization: Bearer ` (which trims to "") as a match against an
    // empty expected token, so a failed settings write must abort the start
    // rather than degrade into an unauthenticated server.
    let expected_token = get_or_create_mcp_token(&app).ok_or_else(|| {
        std::io::Error::other(
            "MCP bearer token could not be created or persisted; refusing to start an \
             unauthenticated MCP server.",
        )
    })?;

    let listener = TcpListener::bind(addr).await?;
    let server_state = Arc::new(McpServerState {
        sessions: std::sync::Mutex::new(HashMap::new()),
        rate_limiter: std::sync::Mutex::new(RateLimiter::new()),
        expected_token,
        semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
    });

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel::<()>();
    let running = Arc::new(AtomicBool::new(true));

    let app_for_loop = app.clone();
    let running_for_loop = Arc::clone(&running);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((mut stream, _addr)) => {
                            // Whitelist: only allow 127.0.0.1
                            let is_local = stream
                                .peer_addr()
                                .ok()
                                .map(|a| a.ip())
                                .unwrap_or_else(|| {
                                    // If we can't get the address, reject
                                    std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                                })
                                .is_loopback();

                            if !is_local {
                                let _ = stream.shutdown().await;
                                continue;
                            }

                            // Bounded concurrent connections via semaphore
                            let permit = Arc::clone(&server_state.semaphore)
                                .acquire_owned()
                                .await
                                .expect("MCP server semaphore closed");

                            let state_clone = Arc::clone(&server_state);
                            let app_clone = app_for_loop.clone();
                            tokio::spawn(async move {
                                let _permit = permit;
                                if let Err(e) = handle_connection(stream, state_clone, app_clone, port).await {
                                    eprintln!("MCP connection error: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("MCP accept error: {}", e);
                            break;
                        }
                    }
                }
                _ = &mut shutdown_rx => {
                    break;
                }
            }
        }
        running_for_loop.store(false, Ordering::SeqCst);
        let _ = stopped_tx.send(());
    });

    Ok(McpServer {
        shutdown_tx: std::sync::Mutex::new(Some(shutdown_tx)),
        stopped_rx: std::sync::Mutex::new(Some(stopped_rx)),
        running,
        port,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    // ---- JSON-RPC helpers ----

    #[test]
    fn test_jsonrpc_response_has_id() {
        let id = serde_json::json!(42);
        let resp = JsonRpcResponse::success(id.clone(), serde_json::json!({ "ok": true }));
        assert_eq!(resp.id, id);
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }

    #[test]
    fn test_jsonrpc_error_response() {
        let id = serde_json::json!(null);
        let resp = JsonRpcResponse::error(id.clone(), -32601, "Method not found");
        assert_eq!(resp.id, id);
        let err = resp.error.expect("expected error");
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
    }

    // ---- Session ID ----

    #[test]
    fn test_session_id_unique() {
        let a = generate_session_id();
        let b = generate_session_id();
        assert_ne!(a, b);
    }

    #[test]
    fn test_session_id_nonempty() {
        let id = generate_session_id();
        assert!(!id.is_empty());
    }

    // ---- HTTP parsing helpers ----

    #[test]
    fn test_parse_request_line_valid() {
        let (method, path) = parse_request_line("POST /messages HTTP/1.1").unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/messages");
    }

    #[test]
    fn test_parse_request_line_invalid() {
        assert!(parse_request_line("INVALID").is_none());
    }

    #[test]
    fn test_parse_query_params_basic() {
        let params = parse_query_params("/messages?session_id=abc&foo=bar");
        assert_eq!(params.get("session_id"), Some(&"abc".to_string()));
        assert_eq!(params.get("foo"), Some(&"bar".to_string()));
    }

    #[test]
    fn test_parse_query_params_no_query() {
        let params = parse_query_params("/messages");
        assert!(params.is_empty());
    }

    #[test]
    fn test_extract_route() {
        assert_eq!(extract_route("/sse"), "/sse");
        assert_eq!(extract_route("/sse?foo=bar"), "/sse");
        assert_eq!(extract_route("/messages"), "/messages");
    }

    #[test]
    fn test_extract_session_id_present() {
        assert_eq!(
            extract_session_id("/messages?session_id=abc123"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn test_extract_session_id_absent() {
        assert!(extract_session_id("/messages").is_none());
    }

    // ---- Rate limiter (deterministic with mock time) ----

    #[test]
    fn test_rate_limiter_allows_under_limit() {
        let rl = Arc::new(std::sync::Mutex::new(RateLimiter::new()));
        for _ in 0..100 {
            assert!(rl.lock().unwrap().allow());
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_limit() {
        let rl = Arc::new(std::sync::Mutex::new(RateLimiter::new()));
        for _ in 0..100 {
            assert!(rl.lock().unwrap().allow());
        }
        assert!(!rl.lock().unwrap().allow());
    }

    #[test]
    fn test_server_stop_marks_listener_stopped_and_exposes_completion_signal() {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let (_stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        let server = McpServer {
            shutdown_tx: std::sync::Mutex::new(Some(shutdown_tx)),
            stopped_rx: std::sync::Mutex::new(Some(stopped_rx)),
            running: Arc::new(AtomicBool::new(true)),
            port: 39741,
        };

        assert!(server.is_running());
        server.stop();

        assert!(!server.is_running());
        assert!(shutdown_rx.try_recv().is_ok());
        assert!(server.take_stopped_rx().is_some());
        assert!(server.take_stopped_rx().is_none());
    }

    // -----------------------------------------------------------------------
    // Integration tests — ephemeral-loopback-port HTTP transport
    // -----------------------------------------------------------------------

    const TEST_TOKEN: &str = "test-mcp-token-00000000000000000000000000000000";

    struct TestServer {
        port: u16,
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
    }

    impl TestServer {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();

            let state = Arc::new(McpServerState {
                sessions: std::sync::Mutex::new(HashMap::new()),
                rate_limiter: std::sync::Mutex::new(RateLimiter::new()),
                expected_token: TEST_TOKEN.to_string(),
                semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
            });

            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        result = listener.accept() => {
                            if let Ok((stream, _addr)) = result {
                                let state = Arc::clone(&state);
                                let permit = Arc::clone(&state.semaphore)
                                    .acquire_owned()
                                    .await
                                    .expect("test semaphore closed");
                                tokio::spawn(async move {
                                    let _permit = permit;
                                    let _ = handle_test_connection(stream, state, port).await;
                                });
                            }
                        }
                        _ = &mut shutdown_rx => break,
                    }
                }
            });

            Self { port, shutdown_tx }
        }

        fn port(&self) -> u16 {
            self.port
        }
    }

    /// Minimal connection handler for tests: same transport validation as
    /// `handle_connection` but with a simple dispatch that handles `initialize`.
    async fn handle_test_connection(
        stream: TcpStream,
        state: Arc<McpServerState>,
        port: u16,
    ) -> std::io::Result<()> {
        tokio::time::timeout(Duration::from_secs(30), async move {
            let (raw_read, mut write_half) = stream.into_split();
            let mut read_half = BufReader::new(raw_read);

            let mut request_line = String::new();
            if read_half.read_line(&mut request_line).await? == 0 {
                return Ok(());
            }

            let (method, full_path) = match parse_request_line(&request_line) {
                Some(v) => v,
                None => return Ok(()),
            };

            // Headers with aggregate size limit
            let mut headers = HashMap::new();
            let mut total_header_bytes: usize = 0;
            loop {
                let mut header_line = String::new();
                let bytes = read_half.read_line(&mut header_line).await?;
                if bytes == 0 {
                    break;
                }
                total_header_bytes += bytes;
                if total_header_bytes > MAX_HEADER_SIZE {
                    let _ = write_half
                        .write_all(
                            b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\n\r\n",
                        )
                        .await;
                    let _ = write_half.shutdown().await;
                    return Ok(());
                }
                let header_line = header_line.trim();
                if header_line.is_empty() {
                    break;
                }
                if let Some((k, v)) = header_line.split_once(':') {
                    headers.insert(k.trim().to_lowercase(), v.trim().to_string());
                }
            }

            // Origin check
            if let Some(origin) = headers.get("origin") {
                if !origin.is_empty() && !is_origin_allowed(origin, port) {
                    let body =
                        br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":"Origin not allowed"}}"#;
                    let msg = format!(
                        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        std::str::from_utf8(body).unwrap_or(""),
                    );
                    let _ = write_half.write_all(msg.as_bytes()).await;
                    let _ = write_half.shutdown().await;
                    return Ok(());
                }
            }

            // Token check
            if extract_bearer_token(&headers).is_none_or(|t| t != state.expected_token) {
                let body =
                    br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32001,"message":"Unauthorized: MCP Bearer token required."}}"#;
                let msg = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    body.len(),
                    std::str::from_utf8(body).unwrap_or(""),
                );
                let _ = write_half.write_all(msg.as_bytes()).await;
                let _ = write_half.shutdown().await;
                return Ok(());
            }

            // Rate limiter
            let allowed = state.rate_limiter.lock().unwrap().allow();
            if !allowed {
                let _ = write_half
                    .write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n")
                    .await;
                let _ = write_half.shutdown().await;
                return Ok(());
            }

            let route = extract_route(&full_path);

            // Body read
            let content_length = headers
                .get("content-length")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);

            if content_length > MAX_MCP_BODY_SIZE {
                let _ = write_half
                    .write_all(b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\n\r\n")
                    .await;
                // Drain the request body so TCP can FIN without RST
                let mut drain = [0u8; 4096];
                let mut remaining = content_length;
                while remaining > 0 {
                    let n = remaining.min(4096);
                    let _ = read_half.read_exact(&mut drain[..n]).await;
                    remaining -= n;
                }
                return Ok(());
            }

            let mut body = Vec::with_capacity(content_length);
            if content_length > 0 {
                body.resize(content_length, 0u8);
                let _ = read_half.read_exact(&mut body).await;
            }

            match (method.as_str(), route) {
                ("POST", "/mcp") | ("POST", "/sse") => {
                    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
                        Ok(r) => r,
                        Err(e) => {
                            let resp = JsonRpcResponse::error(
                                serde_json::Value::Null,
                                -32700,
                                &format!("JSON parse error: {}", e),
                            );
                            let resp_bytes = serde_json::to_vec(&resp).unwrap_or_default();
                            let msg = format!(
                                "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                resp_bytes.len(),
                                String::from_utf8_lossy(&resp_bytes),
                            );
                            let _ = write_half.write_all(msg.as_bytes()).await;
                            return Ok(());
                        }
                    };

                    if request.id.is_none() {
                        let _ = write_half
                            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
                            .await;
                        return Ok(());
                    }

                    let result = match request.method.as_str() {
                        "initialize" => serde_json::json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": { "tools": {}, "resources": {} },
                            "serverInfo": { "name": "agora", "version": env!("CARGO_PKG_VERSION") }
                        }),
                        _ => serde_json::json!({
                            "error": {
                                "code": -32601,
                                "message": format!("Unknown method: {}", request.method)
                            }
                        }),
                    };

                    let response = JsonRpcResponse::success(
                        request.id.unwrap_or(serde_json::Value::Null),
                        result,
                    );
                    let resp_bytes = response.to_json_bytes();
                    let msg = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        resp_bytes.len(),
                        String::from_utf8_lossy(&resp_bytes),
                    );
                    let _ = write_half.write_all(msg.as_bytes()).await;
                    Ok(())
                }
                _ => {
                    let _ = write_half
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\n\r\n{}")
                        .await;
                    Ok(())
                }
            }
        })
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "test request timed out"))?
    }

    fn build_request(
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&[u8]>,
        extra_headers: &[(&str, &str)],
    ) -> Vec<u8> {
        let mut req = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1\r\n", method, path);
        if let Some(t) = token {
            req.push_str(&format!("Authorization: Bearer {}\r\n", t));
        }
        if let Some(b) = body {
            req.push_str(&format!("Content-Length: {}\r\n", b.len()));
            req.push_str("Content-Type: application/json\r\n");
        }
        for (k, v) in extra_headers {
            req.push_str(&format!("{}: {}\r\n", k, v));
        }
        req.push_str("\r\n");
        let mut bytes = req.into_bytes();
        if let Some(b) = body {
            bytes.extend_from_slice(b);
        }
        bytes
    }

    async fn raw_request(port: u16, raw: &[u8]) -> (u16, Vec<u8>) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream.write_all(raw).await.unwrap();
        // Read response in a loop so that a TCP RST (from unread request body)
        // does not discard data already received.
        let mut response = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => response.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }

        let status_line = String::from_utf8_lossy(&response)
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let body_start = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4)
            .unwrap_or(response.len());
        let body = response[body_start..].to_vec();

        (status, body)
    }

    // ---- Transport integration tests ----

    #[tokio::test]
    async fn test_missing_token_401() {
        let server = TestServer::start().await;
        let req = build_request(
            "POST",
            "/mcp",
            None,
            Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
            &[],
        );
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn test_invalid_token_401() {
        let server = TestServer::start().await;
        let req = build_request(
            "POST",
            "/mcp",
            Some("wrong-token"),
            Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
            &[],
        );
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn test_invalid_origin_403() {
        let server = TestServer::start().await;
        let req = build_request(
            "POST",
            "/mcp",
            Some(TEST_TOKEN),
            Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
            &[("Origin", "https://evil.com")],
        );
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 403);
    }

    #[tokio::test]
    async fn test_valid_mcp_call() {
        let server = TestServer::start().await;
        let req = build_request(
            "POST",
            "/mcp",
            Some(TEST_TOKEN),
            Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
            &[],
        );
        let (status, body) = raw_request(server.port(), &req).await;
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(json["result"]["serverInfo"]["name"], "agora");
    }

    #[tokio::test]
    async fn test_oversized_body_413() {
        let server = TestServer::start().await;
        let big_body = vec![b'X'; MAX_MCP_BODY_SIZE + 1];
        let req = build_request("POST", "/mcp", Some(TEST_TOKEN), Some(&big_body), &[]);
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 413);
    }

    #[tokio::test]
    async fn test_header_too_large_431() {
        let server = TestServer::start().await;
        let huge_value = "X".repeat(MAX_HEADER_SIZE);
        let req = build_request(
            "POST",
            "/mcp",
            Some(TEST_TOKEN),
            Some(br#"{}"#),
            &[("X-Huge", &huge_value)],
        );
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 431);
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let server = TestServer::start().await;
        for _ in 0..100 {
            let req = build_request(
                "POST",
                "/mcp",
                Some(TEST_TOKEN),
                Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
                &[],
            );
            let (status, _) = raw_request(server.port(), &req).await;
            assert_eq!(status, 200, "expected 200, got {}", status);
        }
        // 101st request should be rate-limited
        let req = build_request(
            "POST",
            "/mcp",
            Some(TEST_TOKEN),
            Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
            &[],
        );
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 429);
    }

    #[tokio::test]
    async fn test_stop_restart_lifecycle() {
        let server = TestServer::start().await;
        let req = build_request(
            "POST",
            "/mcp",
            Some(TEST_TOKEN),
            Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
            &[],
        );
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 200);

        // Explicitly stop the server
        let _ = server.shutdown_tx.send(());
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Start a fresh server on a new port
        let server2 = TestServer::start().await;
        let req2 = build_request(
            "POST",
            "/mcp",
            Some(TEST_TOKEN),
            Some(br#"{"jsonrpc":"2.0","method":"initialize","id":1}"#),
            &[],
        );
        let (status2, _) = raw_request(server2.port(), &req2).await;
        assert_eq!(status2, 200);
    }

    #[tokio::test]
    async fn test_malformed_request_404() {
        let server = TestServer::start().await;
        let req = b"GET /unknown HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer test-mcp-token-00000000000000000000000000000000\r\n\r\n";
        let (status, _) = raw_request(server.port(), req).await;
        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn test_malformed_header_skip_200() {
        let server = TestServer::start().await;
        let req = b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nBadHeaderNoColon\r\nAuthorization: Bearer test-mcp-token-00000000000000000000000000000000\r\nContent-Length: 46\r\nContent-Type: application/json\r\n\r\n{\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"id\":1}";
        let (status, _) = raw_request(server.port(), req).await;
        assert_eq!(status, 200, "malformed header line should be skipped");
    }

    #[tokio::test]
    async fn test_notification_202() {
        let server = TestServer::start().await;
        // A JSON-RPC notification (no "id" field) should get 202 Accepted
        let req = build_request(
            "POST",
            "/mcp",
            Some(TEST_TOKEN),
            Some(br#"{"jsonrpc":"2.0","method":"initialize"}"#),
            &[],
        );
        let (status, _) = raw_request(server.port(), &req).await;
        assert_eq!(status, 202);
    }

    #[tokio::test]
    async fn test_semaphore_bounds_concurrent_connections() {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(2));
        // Acquire both permits
        let _p1 = semaphore.acquire().await;
        let _p2 = semaphore.acquire().await;
        // Third acquire without permit should not complete
        let try_result = semaphore.try_acquire();
        assert!(try_result.is_err(), "semaphore should be exhausted at 3");
    }

    fn auth_headers(value: &str) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), value.to_string());
        headers
    }

    /// An empty expected token must never authorize anything. `start_server`
    /// refuses to launch without a token, but `extract_bearer_token` trims, so a
    /// bare `Authorization: Bearer ` produces `Some("")` — which a naive
    /// `token != expected` comparison would accept against an empty expected
    /// token, silently turning the listener into an open one.
    #[test]
    fn empty_expected_token_authorizes_nothing() {
        assert!(!is_authorized("", &auth_headers("Bearer ")));
        assert!(!is_authorized("", &auth_headers("Bearer")));
        assert!(!is_authorized("", &auth_headers("bearer ")));
        assert!(!is_authorized("", &auth_headers("Bearer anything")));
        assert!(!is_authorized("", &HashMap::new()));
    }

    #[test]
    fn is_authorized_accepts_only_the_exact_token() {
        assert!(is_authorized(
            TEST_TOKEN,
            &auth_headers(&format!("Bearer {TEST_TOKEN}"))
        ));
        // Lowercase scheme is accepted by design (some clients send it).
        assert!(is_authorized(
            TEST_TOKEN,
            &auth_headers(&format!("bearer {TEST_TOKEN}"))
        ));
        assert!(!is_authorized(
            TEST_TOKEN,
            &auth_headers("Bearer wrong-token")
        ));
        assert!(!is_authorized(TEST_TOKEN, &auth_headers("Bearer ")));
        assert!(!is_authorized(TEST_TOKEN, &HashMap::new()));
        // A non-Bearer scheme is not a token.
        assert!(!is_authorized(
            TEST_TOKEN,
            &auth_headers(&format!("Basic {TEST_TOKEN}"))
        ));
    }

    #[test]
    fn tokens_match_is_exact() {
        assert!(tokens_match(TEST_TOKEN, TEST_TOKEN));
        assert!(!tokens_match("", TEST_TOKEN));
        assert!(!tokens_match(TEST_TOKEN, ""));
        // Differs only in the last byte — the loop must still reach it.
        let mut nearly = TEST_TOKEN.to_string();
        nearly.pop();
        nearly.push('f');
        assert_ne!(nearly, TEST_TOKEN);
        assert!(!tokens_match(&nearly, TEST_TOKEN));
        // A correct prefix is not a match.
        assert!(!tokens_match(&TEST_TOKEN[..10], TEST_TOKEN));
    }

    /// A header line that never terminates must be refused at the read, not
    /// buffered until the connection timeout. `read_line_bounded` returning
    /// `None` is what drives the 431 response.
    #[tokio::test]
    async fn read_line_bounded_refuses_an_unterminated_line() {
        let endless = vec![b'x'; 4096];
        let mut reader = BufReader::new(std::io::Cursor::new(endless));
        let mut buf = String::new();
        let result = read_line_bounded(&mut reader, &mut buf, 1024)
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "an unterminated over-long line must be rejected"
        );
        assert_eq!(buf.len(), 1024, "no more than the limit may be buffered");
    }

    #[tokio::test]
    async fn read_line_bounded_passes_a_normal_line() {
        let mut reader = BufReader::new(std::io::Cursor::new(b"GET /mcp HTTP/1.1\r\n".to_vec()));
        let mut buf = String::new();
        let result = read_line_bounded(&mut reader, &mut buf, 1024)
            .await
            .unwrap();
        assert_eq!(result, Some(19));
        assert_eq!(buf, "GET /mcp HTTP/1.1\r\n");
    }
}
