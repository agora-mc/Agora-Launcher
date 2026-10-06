use crate::{InstallId, LayerId, RelPath, RuntimeIdentity, ToolId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaseMode {
    Linked,
    Copied,
}

impl std::fmt::Display for BaseMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BaseMode::Linked => write!(f, "linked"),
            BaseMode::Copied => write!(f, "copied"),
        }
    }
}

impl std::str::FromStr for BaseMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "linked" => Ok(BaseMode::Linked),
            "copied" => Ok(BaseMode::Copied),
            other => Err(format!(
                "invalid base mode '{other}': expected 'linked' or 'copied'"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BaseReference {
    Pinned {
        id: String,
        runtime: RuntimeIdentity,
        mode: BaseMode,
    },
    Unpinned {
        install: InstallId,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayerSource {
    Base {
        base: BaseReference,
    },
    Content {
        content: String,
    },
    /// Existing Minecraft directories remain in place during migration. These
    /// are references, not a claim that content is already immutable/CAS-backed.
    InstanceContent {
        path: RelPath,
        content_kind: String,
    },
    Generated {
        tool: ToolId,
        generation: String,
        inputs: InputFingerprint,
    },
    Writable {
        path: RelPath,
    },
    Staging {
        tool: ToolId,
        run: String,
        path: RelPath,
    },
}

impl LayerSource {
    /// Rank according to MASTER_SPEC §26.5 layer order (lowest first):
    /// 1. Base (pinned base or unpinned store install)
    /// 2. Content (from content store or instance content)
    /// 3. Generated (tool output)
    /// 4. Writable (game writes)
    /// 5. Staging (active tool run writes)
    pub fn rank(&self) -> u8 {
        match self {
            LayerSource::Base { .. } => 1,
            LayerSource::Content { .. } | LayerSource::InstanceContent { .. } => 2,
            LayerSource::Generated { .. } => 3,
            LayerSource::Writable { .. } => 4,
            LayerSource::Staging { .. } => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum InputFingerprint {
    /// Host digest of enabled content, load order and declared relevant settings.
    Known(String),
    /// Imported output (e.g. MO2 overwrite) must not be claimed current.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub enabled: bool,
    pub mount_path: RelPath,
    #[serde(default, skip_serializing_if = "RelPath::is_empty")]
    pub source_path: RelPath,
    pub source: LayerSource,
    /// Relative paths hidden from lower layers; lower bytes are never deleted.
    #[serde(default)]
    pub whiteouts: Vec<RelPath>,
}

impl Layer {
    /// Generated output is changed only by promotion of a successful staging
    /// run. InstanceContent retains legacy Redirect behaviour; it is not a VFS
    /// lower layer. The host must isolate every VFS lower layer's writes.
    pub fn is_write_target(&self) -> bool {
        matches!(
            self.source,
            LayerSource::Writable { .. } | LayerSource::Staging { .. }
        )
    }
}

/// A layer stack invariant error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayerError {
    #[error("layer {above:?} is above {below:?} but belongs lower in the stack")]
    OutOfOrder { below: LayerId, above: LayerId },
    #[error("more than one {0} layer")]
    Duplicate(&'static str),
    #[error("two layers share the id {0:?}")]
    DuplicateId(LayerId),
    #[error("two generated layers for tool {0}")]
    DuplicateTool(ToolId),
}

/// A validated, ordered stack of layers, lowest first (MASTER_SPEC §26.5).
///
/// Enforces:
/// 1. Stack order: Base (1) <= Content (2) <= Generated (3) <= Writable (4) <= Staging (5)
/// 2. Unique layer IDs across the entire stack.
/// 3. At most one Base layer.
/// 4. At most one Writable layer.
/// 5. At most one Staging layer.
/// 6. At most one Generated layer per tool.
///
/// Enforced on construction, deserialization, and serialization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayerStack(Vec<Layer>);

impl LayerStack {
    pub fn validate(&self) -> Result<(), LayerError> {
        let mut seen_base = false;
        let mut seen_writable = false;
        let mut seen_staging = false;
        let mut seen_tools: Vec<ToolId> = Vec::new();

        for (i, layer) in self.0.iter().enumerate() {
            if let Some(prev) = i.checked_sub(1).map(|p| &self.0[p]) {
                if prev.source.rank() > layer.source.rank() {
                    return Err(LayerError::OutOfOrder {
                        below: prev.id.clone(),
                        above: layer.id.clone(),
                    });
                }
            }
            if self.0[..i].iter().any(|other| other.id == layer.id) {
                return Err(LayerError::DuplicateId(layer.id.clone()));
            }
            match &layer.source {
                LayerSource::Base { .. } => {
                    if seen_base {
                        return Err(LayerError::Duplicate("base"));
                    }
                    seen_base = true;
                }
                LayerSource::Writable { .. } => {
                    if seen_writable {
                        return Err(LayerError::Duplicate("writable"));
                    }
                    seen_writable = true;
                }
                LayerSource::Staging { .. } => {
                    if seen_staging {
                        return Err(LayerError::Duplicate("staging"));
                    }
                    seen_staging = true;
                }
                LayerSource::Generated { tool, .. } => {
                    if seen_tools.contains(tool) {
                        return Err(LayerError::DuplicateTool(tool.clone()));
                    }
                    seen_tools.push(tool.clone());
                }
                LayerSource::Content { .. } | LayerSource::InstanceContent { .. } => {}
            }
        }
        Ok(())
    }

    /// Construct and validate a `LayerStack`.
    pub fn new(layers: Vec<Layer>) -> Result<Self, LayerError> {
        let stack = Self(layers);
        stack.validate()?;
        Ok(stack)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn layers(&self) -> &[Layer] {
        &self.0
    }

    /// The layers to mount for the instance.
    /// Persisted staging layers are records for recovery; leaves staging out unless `run_active` is true.
    pub fn layers_to_mount(&self, run_active: bool) -> impl Iterator<Item = &Layer> {
        self.0
            .iter()
            .filter(move |layer| run_active || !matches!(layer.source, LayerSource::Staging { .. }))
    }

    /// Alias for `layers_to_mount`.
    pub fn mount_layers(&self, run_active: bool) -> impl Iterator<Item = &Layer> {
        self.layers_to_mount(run_active)
    }

    /// Layers that are write targets, lowest first.
    pub fn writable(&self) -> impl Iterator<Item = &Layer> {
        self.0.iter().filter(|layer| layer.is_write_target())
    }
}

impl std::ops::Deref for LayerStack {
    type Target = [Layer];
    fn deref(&self) -> &[Layer] {
        &self.0
    }
}

impl IntoIterator for LayerStack {
    type Item = Layer;
    type IntoIter = std::vec::IntoIter<Layer>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a LayerStack {
    type Item = &'a Layer;
    type IntoIter = std::slice::Iter<'a, Layer>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl TryFrom<Vec<Layer>> for LayerStack {
    type Error = LayerError;
    fn try_from(layers: Vec<Layer>) -> Result<Self, Self::Error> {
        Self::new(layers)
    }
}

impl From<LayerStack> for Vec<Layer> {
    fn from(stack: LayerStack) -> Self {
        stack.0
    }
}

impl Serialize for LayerStack {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(serde::ser::Error::custom)?;
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LayerStack {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let layers = Vec::<Layer>::deserialize(deserializer)?;
        let stack = Self(layers);
        stack.validate().map_err(serde::de::Error::custom)?;
        Ok(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(id: &str, source: LayerSource) -> Layer {
        Layer {
            id: LayerId::new(id).unwrap(),
            enabled: true,
            mount_path: RelPath::default(),
            source_path: RelPath::default(),
            source,
            whiteouts: Vec::new(),
        }
    }

    fn generated(id: &str, tool: &str) -> Layer {
        layer(
            id,
            LayerSource::Generated {
                tool: ToolId::new(tool).unwrap(),
                generation: "1".into(),
                inputs: InputFingerprint::Unknown,
            },
        )
    }

    #[test]
    fn generated_output_is_not_a_write_target() {
        let mut l = generated("nemesis-output", "nemesis");
        assert!(!l.is_write_target());
        l.source = LayerSource::Staging {
            tool: ToolId::new("nemesis").unwrap(),
            run: "run-1".into(),
            path: RelPath::new("staging/run-1").unwrap(),
        };
        assert!(l.is_write_target());
    }

    #[test]
    fn stack_order_and_invariants() {
        let base = layer(
            "base",
            LayerSource::Base {
                base: BaseReference::Unpinned {
                    install: InstallId::new("inst").unwrap(),
                    reason: "test".into(),
                },
            },
        );
        let content = layer(
            "content",
            LayerSource::Content {
                content: "hash".into(),
            },
        );
        let gen_a = generated("gen-a", "tool-a");
        let gen_b = generated("gen-b", "tool-b");
        let writable = layer(
            "writable",
            LayerSource::Writable {
                path: RelPath::new("writes").unwrap(),
            },
        );
        let staging = layer(
            "staging",
            LayerSource::Staging {
                tool: ToolId::new("tool-a").unwrap(),
                run: "1".into(),
                path: RelPath::new("staging/1").unwrap(),
            },
        );

        let stack = LayerStack::new(vec![
            base.clone(),
            content.clone(),
            gen_a.clone(),
            gen_b.clone(),
            writable.clone(),
            staging.clone(),
        ])
        .unwrap();

        // Staging left out unless run is active
        let mounted_idle: Vec<_> = stack
            .layers_to_mount(false)
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(
            mounted_idle,
            ["base", "content", "gen-a", "gen-b", "writable"]
        );

        let mounted_active: Vec<_> = stack.layers_to_mount(true).map(|l| l.id.as_str()).collect();
        assert_eq!(
            mounted_active,
            ["base", "content", "gen-a", "gen-b", "writable", "staging"]
        );

        // Out of order rejected
        assert!(matches!(
            LayerStack::new(vec![writable.clone(), content.clone()]),
            Err(LayerError::OutOfOrder { .. })
        ));

        // Duplicate IDs rejected
        assert!(matches!(
            LayerStack::new(vec![
                content.clone(),
                layer(
                    "content",
                    LayerSource::Writable {
                        path: RelPath::default()
                    }
                )
            ]),
            Err(LayerError::DuplicateId(_))
        ));

        // Duplicate generated tool rejected
        assert!(matches!(
            LayerStack::new(vec![generated("g1", "tool-a"), generated("g2", "tool-a")]),
            Err(LayerError::DuplicateTool(_))
        ));
    }

    #[test]
    fn try_from_validates() {
        let invalid = vec![
            layer(
                "w",
                LayerSource::Writable {
                    path: RelPath::default(),
                },
            ),
            layer(
                "c",
                LayerSource::Content {
                    content: "hash".into(),
                },
            ),
        ];
        assert!(LayerStack::try_from(invalid).is_err());
    }
}
