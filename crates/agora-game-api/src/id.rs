//! Validated identifiers and relative paths for the game contract.
//!
//! Identifiers end up in file names, URLs and manifests. They must be validated
//! on construction and on deserialization.
//!
//! Namespaced identifiers with at most one colon (e.g. `minecraft:mod`) are
//! permitted so legacy and multi-game layers remain compatible.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum length accepted for an identifier.
pub const MAX_ID_LEN: usize = 64;

/// An identifier was empty, too long, or contained disallowed characters.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} id {value:?}: use 1-{MAX_ID_LEN} characters from a-z, 0-9, '-', '_' and at most one ':' namespace separator, starting with a letter or digit")]
pub struct IdError {
    pub kind: &'static str,
    pub value: String,
}

fn is_valid_slug_part(part: &str) -> bool {
    let mut chars = part.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn is_valid_id(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_ID_LEN {
        return false;
    }
    let parts: Vec<&str> = value.split(':').collect();
    match parts.len() {
        1 => is_valid_slug_part(parts[0]),
        2 => is_valid_slug_part(parts[0]) && is_valid_slug_part(parts[1]),
        _ => false,
    }
}

macro_rules! validated_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and construct an identifier.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if is_valid_id(&value) {
                    Ok(Self(value))
                } else {
                    Err(IdError { kind: $kind, value })
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;
            fn try_from(value: String) -> Result<Self, IdError> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdError;
            fn try_from(value: &str) -> Result<Self, IdError> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<String> for $name {
            fn eq(&self, other: &String) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<$name> for str {
            fn eq(&self, other: &$name) -> bool {
                self == other.0.as_str()
            }
        }

        impl PartialEq<$name> for &str {
            fn eq(&self, other: &$name) -> bool {
                *self == other.0.as_str()
            }
        }

        impl PartialEq<$name> for String {
            fn eq(&self, other: &$name) -> bool {
                self == &other.0
            }
        }
    };
}

validated_id!(
    /// A game identifier: `minecraft`, `skyrim`, `valheim`.
    GameId,
    "game"
);

validated_id!(
    /// Where a copy of a game was obtained: `steam`, `gog`, `mojang`,
    /// `microsoft-store`, `epic`, `direct`.
    StoreId,
    "store"
);

validated_id!(
    /// An install discovered on the system.
    InstallId,
    "install"
);

validated_id!(
    /// A modding framework or loader: `fabric`, `forge`, `skse`, `bepinex`.
    FrameworkId,
    "framework"
);

validated_id!(
    /// A declared tool: `nemesis`, `bodyslide`.
    ToolId,
    "tool"
);

validated_id!(
    /// A layer within an instance: `base`, `content`, `minecraft:mod`.
    LayerId,
    "layer"
);

impl GameId {
    pub const MINECRAFT: &'static str = "minecraft";

    pub fn minecraft() -> Self {
        Self(Self::MINECRAFT.to_string())
    }

    pub fn is_minecraft(&self) -> bool {
        self.0 == Self::MINECRAFT
    }
}

impl StoreId {
    pub const MOJANG: &'static str = "mojang";
    pub const STEAM: &'static str = "steam";
    pub const GOG: &'static str = "gog";
    pub const EPIC: &'static str = "epic";
    pub const MICROSOFT_STORE: &'static str = "microsoft-store";
    pub const DIRECT: &'static str = "direct";

    pub fn mojang() -> Self {
        Self(Self::MOJANG.to_string())
    }

    pub fn steam() -> Self {
        Self(Self::STEAM.to_string())
    }

    pub fn gog() -> Self {
        Self(Self::GOG.to_string())
    }

    pub fn epic() -> Self {
        Self(Self::EPIC.to_string())
    }

    pub fn microsoft_store() -> Self {
        Self(Self::MICROSOFT_STORE.to_string())
    }

    pub fn direct() -> Self {
        Self(Self::DIRECT.to_string())
    }
}

/// A validated path relative to a game, instance or base root.
///
/// Ensures the path cannot escape its root: backslashes are normalized to `/`,
/// `..` components are rejected, drive letters and absolute paths are rejected,
/// and null bytes are rejected. Empty string `""` represents the root directory itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RelPath(String);

/// A relative path was absolute, contained a drive letter, or attempted to leave its root.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid relative path {0:?}: must be relative and stay inside its root, no '..', no drive letters, no absolute paths")]
pub struct RelPathError(pub String);

impl RelPath {
    pub fn new(value: impl Into<String>) -> Result<Self, RelPathError> {
        let value = value.into();
        let normalized = value.replace('\\', "/");
        if normalized.starts_with('/')
            || normalized.contains('\0')
            || normalized.contains(':')
            || normalized.split('/').any(|part| part == "..")
        {
            return Err(RelPathError(value));
        }
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RelPath {
    type Error = RelPathError;
    fn try_from(value: String) -> Result<Self, RelPathError> {
        Self::new(value)
    }
}

impl TryFrom<&str> for RelPath {
    type Error = RelPathError;
    fn try_from(value: &str) -> Result<Self, RelPathError> {
        Self::new(value)
    }
}

impl From<RelPath> for String {
    fn from(path: RelPath) -> String {
        path.0
    }
}

impl fmt::Display for RelPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for RelPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for RelPath {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for RelPath {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for RelPath {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for RelPath {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for RelPath {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl PartialEq<RelPath> for str {
    fn eq(&self, other: &RelPath) -> bool {
        self == other.0.as_str()
    }
}

impl PartialEq<RelPath> for &str {
    fn eq(&self, other: &RelPath) -> bool {
        *self == other.0.as_str()
    }
}

impl PartialEq<RelPath> for String {
    fn eq(&self, other: &RelPath) -> bool {
        self == &other.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_reject_invalid_slugs() {
        let too_long = "x".repeat(65);
        for bad in [
            "",
            "..",
            "a/b",
            "a\\b",
            "A",
            "has space",
            "-lead",
            "a.b",
            "\u{e9}",
            ":",
            ":mod",
            "minecraft:",
            "a:b:c",
            &too_long,
        ] {
            assert!(GameId::new(bad).is_err(), "{bad:?} must be rejected");
            assert!(LayerId::new(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn ids_accept_valid_slugs_and_namespaces() {
        let longest = "x".repeat(64);
        for good in [
            "minecraft",
            "skyrim-se",
            "7days",
            "a_b",
            "minecraft:mod",
            "minecraft:resourcepack",
            &longest,
        ] {
            assert!(GameId::new(good).is_ok(), "{good:?} must be accepted");
            assert!(LayerId::new(good).is_ok(), "{good:?} must be accepted");
        }
    }

    #[test]
    fn rel_path_validation_and_normalization() {
        for bad in [
            "/abs", "\\abs", "../x", "a/../b", "a\\..\\b", "C:/x", "D:\\x", "a\0b",
        ] {
            assert!(RelPath::new(bad).is_err(), "{bad:?} must be rejected");
        }

        assert_eq!(RelPath::new("").unwrap().as_str(), "");
        assert_eq!(
            RelPath::new("Data\\SKSE\\Plugins").unwrap().as_str(),
            "Data/SKSE/Plugins"
        );
        assert_eq!(
            RelPath::new("Data/SKSE/Plugins").unwrap().as_str(),
            "Data/SKSE/Plugins"
        );
        assert_eq!(RelPath::new("a..b/c").unwrap().as_str(), "a..b/c");
    }

    #[test]
    fn try_from_validates_newtypes() {
        assert!(GameId::try_from("../etc".to_string()).is_err());
        assert_eq!(
            GameId::try_from("minecraft".to_string()).unwrap(),
            GameId::minecraft()
        );
        assert_eq!(
            LayerId::try_from("minecraft:mod".to_string())
                .unwrap()
                .as_str(),
            "minecraft:mod"
        );
        assert!(RelPath::try_from("../escape".to_string()).is_err());
        assert_eq!(
            RelPath::try_from("mods\\sub".to_string()).unwrap().as_str(),
            "mods/sub"
        );
    }
}
