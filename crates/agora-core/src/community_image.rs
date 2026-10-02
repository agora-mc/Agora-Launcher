//! Images embedded in community-written About text.
//!
//! Project descriptions link pictures from anywhere (badge services, personal
//! hosts, image CDNs). The webview's CSP only admits a few first-party image
//! hosts, so core fetches the rest, from any public host, and hands the page a
//! `data:` URL — but only after the bytes have been identified as an image by
//! their signature. Whatever the server calls the response, something that is
//! not a recognised image format never reaches the page.
//!
//! The page renders the result with `<img>`, where an SVG's scripts and
//! external references do not run, so SVG is accepted like the raster formats.

use base64::Engine;

use crate::ctx::Ctx;
use crate::error::{LauncherError, LauncherResult};
use crate::http_client::{self, ClientCategory, HostPolicy};

/// Largest image fetched for an About page.
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

/// A fetched image whose format was confirmed from its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityImage {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

impl CommunityImage {
    pub fn to_data_url(&self) -> String {
        format!(
            "data:{};base64,{}",
            self.mime,
            base64::engine::general_purpose::STANDARD.encode(&self.bytes)
        )
    }
}

/// Fetch an image from any public HTTPS host and confirm it is an image.
pub async fn fetch(ctx: &Ctx, url: &str) -> LauncherResult<CommunityImage> {
    let bytes = http_client::checked_get_bytes_with_policy(
        &ctx.http_clients,
        ClientCategory::CommunityImage,
        url,
        HostPolicy::AnyPublicHost,
    )
    .await?;
    let mime = sniff_image(&bytes).ok_or_else(|| LauncherError::Generic {
        code: "ERR_NOT_AN_IMAGE".into(),
        message: "The linked file is not an image.".into(),
    })?;
    Ok(CommunityImage { mime, bytes })
}

/// Identify an image format from its leading bytes.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis") {
        return Some("image/avif");
    }
    if bytes.starts_with(b"BM") && bytes.len() >= 14 {
        return Some("image/bmp");
    }
    if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() >= 6 {
        return Some("image/x-icon");
    }
    if is_svg(bytes) {
        return Some("image/svg+xml");
    }
    None
}

/// Whether the document's root element is `<svg>`, after an optional BOM,
/// XML declaration, comments, doctype and whitespace.
fn is_svg(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut rest = text.strip_prefix('\u{feff}').unwrap_or(text);
    loop {
        rest = rest.trim_start();
        let skipped = if rest.starts_with("<?") {
            rest.find("?>").map(|end| &rest[end + 2..])
        } else if rest.starts_with("<!--") {
            rest.find("-->").map(|end| &rest[end + 3..])
        } else if rest.starts_with("<!") {
            rest.find('>').map(|end| &rest[end + 1..])
        } else {
            break;
        };
        match skipped {
            Some(next) => rest = next,
            None => return false,
        }
    }
    rest.strip_prefix("<svg").is_some_and(|after| {
        after
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || c == '>' || c == '/')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_raster_signatures() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a...."), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"\0\0\0\x1cftypavif\0\0"), Some("image/avif"));
    }

    #[test]
    fn recognises_svg_after_prolog() {
        let svg = "\u{feff}<?xml version=\"1.0\"?>\n<!-- badge -->\n<!DOCTYPE svg>\n<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
        assert_eq!(sniff_image(svg.as_bytes()), Some("image/svg+xml"));
        assert_eq!(sniff_image(b"<svg>"), Some("image/svg+xml"));
    }

    #[test]
    fn rejects_non_images() {
        assert_eq!(sniff_image(b"<!doctype html><html><body>"), None);
        assert_eq!(sniff_image(b"<svgfoo>"), None);
        assert_eq!(sniff_image(b"<?xml version=\"1.0\"?><rss>"), None);
        assert_eq!(sniff_image(b"MZ\x90\0"), None);
        assert_eq!(sniff_image(b"PK\x03\x04"), None);
        assert_eq!(sniff_image(b"<!-- unterminated"), None);
        assert_eq!(sniff_image(b""), None);
    }

    #[test]
    fn any_public_host_is_scoped_to_images() {
        assert!(http_client::check_request_url_with_policy(
            ClientCategory::CommunityImage,
            "https://badges.example/a.svg",
            HostPolicy::AnyPublicHost,
        )
        .is_ok());
        assert!(http_client::check_request_url_with_policy(
            ClientCategory::Modrinth,
            "https://badges.example/a.svg",
            HostPolicy::AnyPublicHost,
        )
        .is_err());
        assert!(http_client::check_request_url_with_policy(
            ClientCategory::CommunityImage,
            "http://badges.example/a.svg",
            HostPolicy::AnyPublicHost,
        )
        .is_err());
    }

    #[test]
    fn data_url_carries_the_sniffed_type() {
        let image = CommunityImage {
            mime: "image/png",
            bytes: vec![1, 2, 3],
        };
        assert_eq!(image.to_data_url(), "data:image/png;base64,AQID");
    }
}
