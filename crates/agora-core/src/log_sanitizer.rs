use regex::Regex;
use std::sync::OnceLock;

/// Matches a Windows user profile path on any drive, in any case. Minecraft and
/// mod loaders print paths in whatever case the JVM handed them, so an
/// anchored, case-sensitive `C:\Users\` misses `c:\users\` and `D:\Users\`.
fn windows_user_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)([a-z]):\\Users\\([^\\]+)").unwrap())
}

fn linux_home_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)/home/([^/]+)").unwrap())
}

fn macos_home_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)/Users/([^/]+)").unwrap())
}

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[a-f0-9]{32,}\b").unwrap())
}

pub fn sanitize_log(input: &str) -> String {
    // `${1}` preserves the original drive letter; only the user name is dropped.
    let s = windows_user_re().replace_all(input, r"${1}:\Users\<user>");
    let s = linux_home_re().replace_all(&s, r"/home/<user>");
    let s = macos_home_re().replace_all(&s, r"/Users/<user>");
    let s = token_re().replace_all(&s, "<redacted>");
    s.to_string()
}

pub fn sanitize_log_lines(input: &str) -> String {
    input
        .lines()
        .map(sanitize_log)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Sanitize `input` using the standard rules and also replace every non-empty
/// known secret with `[REDACTED]`. This handles arbitrary opaque/JWT/base64
/// tokens without relying on regex shape assumptions.
///
/// Empty-string secrets are silently ignored to avoid blanket redaction.
pub fn sanitize_log_with_secrets(input: &str, secrets: &[&str]) -> String {
    let mut s = sanitize_log(input);
    for secret in secrets {
        if !secret.is_empty() {
            s = s.replace(secret, "[REDACTED]");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_windows_user() {
        let input = r"Loading mod from C:\Users\JohnDoe\.minecraft\mods";
        let result = sanitize_log(input);
        assert!(!result.contains("JohnDoe"));
        assert!(result.contains(r"C:\Users\<user>"));
    }

    #[test]
    fn test_sanitize_windows_user_any_drive_and_case() {
        for input in [
            r"c:\users\JohnDoe\.minecraft\mods",
            r"D:\Users\JohnDoe\.minecraft\mods",
            r"C:\USERS\JohnDoe\.minecraft\mods",
        ] {
            let result = sanitize_log(input);
            assert!(
                !result.contains("JohnDoe"),
                "user name survived sanitization of {input}: {result}"
            );
        }
        // The drive letter is not privacy-relevant and is preserved.
        assert!(sanitize_log(r"D:\Users\JohnDoe\x").starts_with(r"D:\Users\<user>"));
    }

    #[test]
    fn test_sanitize_linux_home() {
        let input = "Loading mod from /home/john/.minecraft/mods";
        let result = sanitize_log(input);
        assert!(!result.contains("/home/john"));
        assert!(result.contains("/home/<user>"));
    }

    #[test]
    fn test_sanitize_macos_home() {
        let input = "Loading mod from /Users/john/.minecraft/mods";
        let result = sanitize_log(input);
        assert!(!result.contains("/Users/john"));
        assert!(result.contains("/Users/<user>"));
    }

    #[test]
    fn test_sanitize_token() {
        let input = "token: abc123def456789012345678901234567890";
        let result = sanitize_log(input);
        assert!(result.contains("<redacted>"));
    }

    #[test]
    fn test_sanitize_multiple_lines() {
        let input = "line1\nline2";
        let result = sanitize_log_lines(input);
        assert_eq!(result, "line1\nline2");
    }

    #[test]
    fn test_sanitize_does_not_alter_normal_text() {
        let input = "This is a normal log line with no PII";
        let result = sanitize_log(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_sanitize_with_secrets_redacts_arbitrary_token() {
        let token = "eyJhbGciOiJIUzI1NiJ9.eyJ4dWlkIjoiMjUzNTQzMjM0NTY3ODkwMSJ9.abcd1234+5678/90";
        let input = format!("Got token: {token}");
        let result = sanitize_log_with_secrets(&input, &[token]);
        assert!(!result.contains(token), "token should not appear in output");
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_with_secrets_skips_empty() {
        let input = "Some text";
        let result = sanitize_log_with_secrets(input, &[""]);
        assert_eq!(result, "Some text");
    }

    #[test]
    fn test_sanitize_with_secrets_still_redacts_users() {
        let input = r"Loading mod from C:\Users\JohnDoe\.minecraft\mods";
        let result = sanitize_log_with_secrets(input, &[]);
        assert!(!result.contains("JohnDoe"));
        assert!(result.contains(r"C:\Users\<user>"));
    }

    #[test]
    fn test_sanitize_with_secrets_multiple_secrets() {
        let token1 = "abc123";
        let token2 = "xyz789";
        let input = format!("token1={token1} token2={token2} normal");
        let result = sanitize_log_with_secrets(&input, &[token1, token2]);
        assert!(!result.contains("abc123"));
        assert!(!result.contains("xyz789"));
        assert_eq!(result, "token1=[REDACTED] token2=[REDACTED] normal");
    }
}
