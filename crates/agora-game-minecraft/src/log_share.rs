//! Share a game log as a link, for asking for help in a support channel.
//!
//! The user picks one of the instance's logs, sees exactly the text that will
//! leave the machine, and only then uploads it to mclo.gs, the paste service
//! the Minecraft community already uses. User folder names and anything shaped
//! like a token are removed first, as is the signed-in account's own token.

use agora_core::ctx::Ctx;
use agora_core::error::{LauncherError, LauncherResult};
use agora_core::http_client::ClientCategory;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const UPLOAD_URL: &str = "https://api.mclo.gs/1/log";
/// mclo.gs keeps at most this many lines; the newest are the useful ones.
const MAX_LINES: usize = 25_000;
const MAX_BYTES: usize = 10 * 1024 * 1024;
const CRASH_REPORTS_SHOWN: usize = 5;

/// A log the instance has that can be shared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareableLog {
    /// Path inside the instance, `logs/latest.log` or `crash-reports/<file>`.
    pub path: String,
    pub label: String,
    pub size_bytes: u64,
    pub modified: Option<String>,
}

/// The exact text an upload would send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogPreview {
    pub path: String,
    pub text: String,
    /// Lines left off the start to fit mclo.gs's limit.
    pub dropped_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedLog {
    pub url: String,
}

fn instance_dir(ctx: &Ctx, instance_id: &str) -> LauncherResult<PathBuf> {
    let id = agora_core::paths::sanitize_id(instance_id);
    agora_core::app_paths::validate_path_component(&id)?;
    ctx.paths.instance_dir(&id)
}

fn describe(dir: &Path, relative: &str, label: String) -> Option<ShareableLog> {
    let meta = std::fs::metadata(dir.join(relative)).ok()?;
    meta.is_file().then(|| ShareableLog {
        path: relative.to_string(),
        label,
        size_bytes: meta.len(),
        modified: meta
            .modified()
            .ok()
            .map(|time| chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()),
    })
}

/// The latest game log and the newest crash reports.
pub fn shareable_logs(ctx: &Ctx, instance_id: &str) -> LauncherResult<Vec<ShareableLog>> {
    let dir = instance_dir(ctx, instance_id)?;
    let mut logs: Vec<ShareableLog> = describe(&dir, "logs/latest.log", "Latest game log".into())
        .into_iter()
        .collect();
    let mut reports: Vec<ShareableLog> = std::fs::read_dir(dir.join("crash-reports"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.ends_with(".txt") || !agora_core::app_paths::is_safe_file_name(&name) {
                return None;
            }
            describe(
                &dir,
                &format!("crash-reports/{name}"),
                format!("Crash report {name}"),
            )
        })
        .collect();
    reports.sort_by(|a, b| b.modified.cmp(&a.modified));
    logs.extend(reports.into_iter().take(CRASH_REPORTS_SHOWN));
    Ok(logs)
}

/// What uploading `path` would send. Only paths [`shareable_logs`] offers are
/// accepted, so nothing else in the instance can be read this way.
pub fn preview(ctx: &Ctx, instance_id: &str, path: &str) -> LauncherResult<LogPreview> {
    let dir = instance_dir(ctx, instance_id)?;
    if !shareable_logs(ctx, instance_id)?
        .iter()
        .any(|log| log.path == path)
    {
        return Err(LauncherError::Generic {
            code: "ERR_LOG_NOT_SHAREABLE".into(),
            message: "That file is not one of this instance's shareable logs.".into(),
        });
    }
    let bytes = std::fs::read(dir.join(path)).map_err(|e| LauncherError::Generic {
        code: "ERR_LOG_READ".into(),
        message: format!("Could not read the log: {e}"),
    })?;
    let raw = String::from_utf8_lossy(&bytes);
    let token = crate::msa::load_credentials()
        .ok()
        .flatten()
        .map(|creds| creds.access_token)
        .unwrap_or_default();
    let clean = agora_core::log_sanitizer::sanitize_log_with_secrets(&raw, &[token.as_str()]);
    let (text, dropped_lines) = keep_tail(&clean);
    Ok(LogPreview {
        path: path.to_string(),
        text,
        dropped_lines,
    })
}

/// Keep the newest lines that fit mclo.gs's line and size limits.
fn keep_tail(text: &str) -> (String, usize) {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut bytes = 0;
    for line in lines.iter().rev() {
        if kept.len() == MAX_LINES || bytes + line.len() + 1 > MAX_BYTES {
            break;
        }
        bytes += line.len() + 1;
        kept.push(*line);
    }
    let dropped = lines.len() - kept.len();
    kept.reverse();
    (kept.join("\n"), dropped)
}

/// Upload the previewed text and return its link.
pub async fn upload(ctx: &Ctx, instance_id: &str, path: &str) -> LauncherResult<SharedLog> {
    let preview = preview(ctx, instance_id, path)?;
    if preview.text.trim().is_empty() {
        return Err(LauncherError::Generic {
            code: "ERR_LOG_EMPTY".into(),
            message: "That log is empty.".into(),
        });
    }
    let response = agora_core::http_client::checked_post_form(
        &ctx.http_clients,
        ClientCategory::LogUpload,
        UPLOAD_URL,
        &[("content", preview.text.as_str())],
        &[],
    )
    .await?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.map_err(|e| LauncherError::Generic {
        code: "ERR_LOG_UPLOAD".into(),
        message: format!("mclo.gs sent an unreadable reply ({status}): {e}"),
    })?;
    parse_reply(&body)
}

fn parse_reply(body: &serde_json::Value) -> LauncherResult<SharedLog> {
    match body.get("url").and_then(|url| url.as_str()) {
        Some(url)
            if body.get("success").and_then(|s| s.as_bool()) == Some(true)
                && url.starts_with("https://mclo.gs/") =>
        {
            Ok(SharedLog {
                url: url.to_string(),
            })
        }
        _ => Err(LauncherError::Generic {
            code: "ERR_LOG_UPLOAD".into(),
            message: format!(
                "mclo.gs did not accept the log: {}",
                body.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("no reason given")
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Ctx) {
        let root = tempfile::tempdir().unwrap();
        let ctx = Ctx::for_testing(root.path().to_path_buf());
        let dir = ctx.paths.instance_dir("inst").unwrap();
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::create_dir_all(dir.join("crash-reports")).unwrap();
        std::fs::write(
            dir.join("logs/latest.log"),
            "Loading from C:\\Users\\Alice\\AppData\nsession 0123456789abcdef0123456789abcdef\n",
        )
        .unwrap();
        std::fs::write(dir.join("crash-reports/crash-1.txt"), "boom").unwrap();
        std::fs::write(dir.join("options.txt"), "secret").unwrap();
        (root, ctx)
    }

    #[test]
    fn offers_the_latest_log_and_crash_reports_only() {
        let (_root, ctx) = fixture();
        let logs = shareable_logs(&ctx, "inst").unwrap();
        let paths: Vec<_> = logs.iter().map(|log| log.path.as_str()).collect();
        assert_eq!(paths, vec!["logs/latest.log", "crash-reports/crash-1.txt"]);
        assert!(preview(&ctx, "inst", "options.txt").is_err());
        assert!(preview(&ctx, "inst", "../../etc/passwd").is_err());
    }

    #[test]
    fn preview_removes_user_names_and_tokens() {
        let (_root, ctx) = fixture();
        let preview = preview(&ctx, "inst", "logs/latest.log").unwrap();
        assert!(!preview.text.contains("Alice"), "{}", preview.text);
        assert!(!preview.text.contains("0123456789abcdef0123456789abcdef"));
        assert_eq!(preview.dropped_lines, 0);
    }

    #[test]
    fn long_logs_keep_their_newest_lines() {
        let text = (0..MAX_LINES + 10)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let (kept, dropped) = keep_tail(&text);
        assert_eq!(dropped, 10);
        assert!(kept.starts_with("10\n"));
        assert!(kept.ends_with(&(MAX_LINES + 9).to_string()));
    }

    #[test]
    fn reply_must_be_a_mclogs_link() {
        let ok = serde_json::json!({"success": true, "id": "abc", "url": "https://mclo.gs/abc"});
        assert_eq!(parse_reply(&ok).unwrap().url, "https://mclo.gs/abc");
        let elsewhere = serde_json::json!({"success": true, "url": "https://evil.example/abc"});
        assert!(parse_reply(&elsewhere).is_err());
        let refused = serde_json::json!({"success": false, "error": "Too large"});
        assert!(parse_reply(&refused)
            .unwrap_err()
            .to_string()
            .contains("Too large"));
    }
}
