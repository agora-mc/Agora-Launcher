//! Server status for the multiplayer list: online or not, players, version,
//! message of the day and latency.
//!
//! Speaks the Server List Ping that the game's own Multiplayer screen uses
//! (Minecraft 1.7 and later): a handshake asking for status, a status request
//! answered with JSON, then a ping/pong timed for latency. Older servers and
//! hosts reachable only through a DNS `SRV` record show as unreachable.
//!
//! The request goes through the network gate as `ClientCategory::GameServer`,
//! so Lockdown Mode and its Privacy switch stop it.

use crate::error::LauncherResult;
use crate::http_client::ClientCategory;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TIMEOUT: Duration = Duration::from_secs(5);
/// A status response is a few kilobytes plus a favicon; anything bigger is not
/// a Minecraft server worth reading.
const MAX_RESPONSE: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub online: bool,
    pub latency_ms: Option<u64>,
    pub players_online: Option<i64>,
    pub players_max: Option<i64>,
    pub version: Option<String>,
    /// Message of the day as plain text, formatting codes removed.
    pub motd: Option<String>,
    pub icon_data_url: Option<String>,
    /// Why it could not be reached, when it could not.
    pub error: Option<String>,
}

impl ServerStatus {
    fn offline(error: impl Into<String>) -> Self {
        Self {
            online: false,
            latency_ms: None,
            players_online: None,
            players_max: None,
            version: None,
            motd: None,
            icon_data_url: None,
            error: Some(error.into()),
        }
    }
}

/// Ping one server. An unreachable server is a normal answer (`online:
/// false`), not an error; an error means the request was not allowed.
pub async fn ping(address: &str) -> LauncherResult<ServerStatus> {
    crate::network_gate::authorize(ClientCategory::GameServer)?;
    Ok(status(address).await)
}

async fn status(address: &str) -> ServerStatus {
    let (host, port) = crate::launch_planner::split_server_address(address);
    if host.is_empty() {
        return ServerStatus::offline("No address.");
    }
    let port = match port.map(|p| p.parse::<u16>()) {
        None => 25565,
        Some(Ok(port)) => port,
        Some(Err(_)) => return ServerStatus::offline("The port is not a number."),
    };
    match tokio::time::timeout(TIMEOUT * 2, exchange(&host, port)).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => ServerStatus::offline(error.to_string()),
        Err(_) => ServerStatus::offline("No answer in time."),
    }
}

async fn exchange(host: &str, port: u16) -> std::io::Result<ServerStatus> {
    let mut stream = tokio::time::timeout(TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "Connection timed out.")
        })??;
    stream.set_nodelay(true)?;

    // Handshake: protocol version -1 ("just asking"), next state 1 (status).
    let mut handshake = Vec::new();
    write_varint(&mut handshake, 0x00);
    write_varint(&mut handshake, -1);
    write_string(&mut handshake, host);
    handshake.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut handshake, 1);
    send_packet(&mut stream, &handshake).await?;
    send_packet(&mut stream, &[0x00]).await?;

    let body = read_packet(&mut stream).await?;
    let mut cursor = body.as_slice();
    if read_varint(&mut cursor)? != 0x00 {
        return Err(invalid(
            "Unexpected reply; this may not be a Minecraft server.",
        ));
    }
    let length = read_varint(&mut cursor)? as usize;
    if length > cursor.len() {
        return Err(invalid("Truncated status reply."));
    }
    let json: serde_json::Value = serde_json::from_slice(&cursor[..length])
        .map_err(|_| invalid("The status reply is not valid JSON."))?;

    // Latency: time a ping/pong. A server that answers status but not ping
    // is still online.
    let started = Instant::now();
    let mut ping = vec![0x01];
    ping.extend_from_slice(&42i64.to_be_bytes());
    let latency_ms = match send_packet(&mut stream, &ping).await {
        Ok(()) => tokio::time::timeout(TIMEOUT, read_packet(&mut stream))
            .await
            .ok()
            .and_then(Result::ok)
            .map(|_| started.elapsed().as_millis() as u64),
        Err(_) => None,
    };

    Ok(parse_status(&json, latency_ms))
}

fn parse_status(json: &serde_json::Value, latency_ms: Option<u64>) -> ServerStatus {
    let players = json.get("players");
    let icon = json
        .get("favicon")
        .and_then(|value| value.as_str())
        .filter(|icon| icon.starts_with("data:image/png;base64,") && icon.len() < MAX_RESPONSE);
    ServerStatus {
        online: true,
        latency_ms,
        players_online: players
            .and_then(|p| p.get("online"))
            .and_then(|v| v.as_i64()),
        players_max: players.and_then(|p| p.get("max")).and_then(|v| v.as_i64()),
        version: json
            .get("version")
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .map(strip_formatting),
        motd: json.get("description").map(|d| {
            let mut text = String::new();
            flatten_chat(d, &mut text);
            strip_formatting(text.trim())
        }),
        icon_data_url: icon.map(str::to_string),
        error: None,
    }
}

/// Text of a chat component: a string, or `text` plus `extra` children.
fn flatten_chat(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(text) => out.push_str(text),
        serde_json::Value::Array(parts) => parts.iter().for_each(|part| flatten_chat(part, out)),
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|t| t.as_str()) {
                out.push_str(text);
            }
            if let Some(extra) = map.get("extra") {
                flatten_chat(extra, out);
            }
        }
        _ => {}
    }
}

/// Drop `§x` formatting codes.
fn strip_formatting(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

fn invalid(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.to_string())
}

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut value = value as u32;
    loop {
        if value & !0x7F == 0 {
            out.push(value as u8);
            return;
        }
        out.push((value & 0x7F) as u8 | 0x80);
        value >>= 7;
    }
}

fn read_varint(input: &mut &[u8]) -> std::io::Result<i32> {
    let mut result = 0u32;
    for shift in 0..5 {
        let (&byte, rest) = input
            .split_first()
            .ok_or_else(|| invalid("Truncated reply."))?;
        *input = rest;
        result |= u32::from(byte & 0x7F) << (7 * shift);
        if byte & 0x80 == 0 {
            return Ok(result as i32);
        }
    }
    Err(invalid("Malformed reply."))
}

fn write_string(out: &mut Vec<u8>, text: &str) {
    write_varint(out, text.len() as i32);
    out.extend_from_slice(text.as_bytes());
}

async fn send_packet(stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    let mut packet = Vec::with_capacity(body.len() + 5);
    write_varint(&mut packet, body.len() as i32);
    packet.extend_from_slice(body);
    stream.write_all(&packet).await
}

async fn read_packet(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut length = 0u32;
    for shift in 0..5 {
        let byte = stream.read_u8().await?;
        length |= u32::from(byte & 0x7F) << (7 * shift);
        if byte & 0x80 == 0 {
            break;
        }
        if shift == 4 {
            return Err(invalid("Malformed reply."));
        }
    }
    let length = length as usize;
    if length == 0 || length > MAX_RESPONSE {
        return Err(invalid("The reply is too large to be a server status."));
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-connection fake server that answers status and ping.
    async fn fake_server(json: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let handshake = read_packet(&mut socket).await.unwrap();
            let mut cursor = handshake.as_slice();
            assert_eq!(read_varint(&mut cursor).unwrap(), 0x00);
            assert_eq!(read_varint(&mut cursor).unwrap(), -1);
            assert_eq!(read_packet(&mut socket).await.unwrap(), vec![0x00]);
            let mut reply = Vec::new();
            write_varint(&mut reply, 0x00);
            write_string(&mut reply, json);
            send_packet(&mut socket, &reply).await.unwrap();
            let ping = read_packet(&mut socket).await.unwrap();
            send_packet(&mut socket, &ping).await.unwrap();
        });
        port
    }

    #[tokio::test]
    async fn reads_players_version_motd_and_latency() {
        let port = fake_server(
            r#"{"version":{"name":"Paper 1.21.1","protocol":767},
                "players":{"max":100,"online":7},
                "description":{"text":"§aWelcome ","extra":[{"text":"home"}]},
                "favicon":"data:image/png;base64,iVBOR"}"#,
        )
        .await;
        let status = status(&format!("127.0.0.1:{port}")).await;
        assert!(status.online, "{status:?}");
        assert_eq!(status.players_online, Some(7));
        assert_eq!(status.players_max, Some(100));
        assert_eq!(status.version.as_deref(), Some("Paper 1.21.1"));
        assert_eq!(status.motd.as_deref(), Some("Welcome home"));
        assert!(status.icon_data_url.is_some());
        assert!(status.latency_ms.is_some());
    }

    #[tokio::test]
    async fn an_unreachable_server_is_offline_not_an_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let status = status(&format!("127.0.0.1:{port}")).await;
        assert!(!status.online);
        assert!(status.error.is_some());
    }

    #[test]
    fn varints_round_trip() {
        for value in [0, 1, 127, 128, 25565, -1, i32::MAX] {
            let mut bytes = Vec::new();
            write_varint(&mut bytes, value);
            assert_eq!(read_varint(&mut bytes.as_slice()).unwrap(), value);
        }
    }
}
