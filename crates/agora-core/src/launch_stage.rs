//! Launch-stage vocabulary shared by every adapter: file-preparation counts and
//! the "game finished loading" readiness signal read from the game's own output.

/// Which group of launch files a [`FileProgress`] update describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    ClientJar,
    Libraries,
    Assets,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::ClientJar => "client",
            FileKind::Libraries => "libraries",
            FileKind::Assets => "assets",
        }
    }
}

/// Count of launch files verified (present and hash-checked, or downloaded)
/// so far within one [`FileKind`] group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileProgress {
    pub kind: FileKind,
    pub done: usize,
    pub total: usize,
}

/// Callback used by the materializer to report [`FileProgress`].
pub type FileProgressFn<'a> = dyn Fn(FileProgress) + Send + Sync + 'a;

/// Detects that the game client has finished loading, from its console output.
///
/// There is no launcher-visible "main menu shown" event, so this reads the
/// vanilla client's own log lines, which every loader keeps because Fabric,
/// Quilt, Forge and NeoForge all run the vanilla client and forward its log4j
/// output unchanged:
///
/// * `Sound engine started` — logged by the client once the sound system is
///   up, during the first resource reload, i.e. after the window exists and
///   shortly before the title screen (Minecraft 1.14 and later).
/// * `Created: WxH... -atlas` — logged when a texture atlas is stitched, which
///   happens in the same reload (Minecraft 1.14 and later).
///
/// Earlier lines such as `Setting user:` are printed before the window opens,
/// so they are deliberately not used. The signal can be a few seconds earlier
/// than the title screen, and versions that log neither line never report
/// ready; callers must treat "never ready" as "still loading", not as failure.
#[derive(Debug, Default)]
pub struct ReadinessDetector {
    ready: bool,
}

impl ReadinessDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one output line. Returns true exactly once, on the first line that
    /// indicates the client finished loading.
    pub fn observe(&mut self, line: &str) -> bool {
        if self.ready || !is_ready_line(line) {
            return false;
        }
        self.ready = true;
        true
    }
}

fn is_ready_line(line: &str) -> bool {
    // Only the render thread logs these; requiring it keeps a mod or server
    // chat line that merely quotes the text from tripping the signal.
    if !line.contains("Render thread") && !line.contains("Client thread") {
        return false;
    }
    line.contains("Sound engine started") || (line.contains("Created: ") && line.contains("-atlas"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_lines_do_not_count() {
        let mut d = ReadinessDetector::new();
        assert!(!d.observe("[12:00:00] [Render thread/INFO]: Setting user: Steve"));
        assert!(!d.observe("[12:00:01] [Render thread/INFO]: Backend library: LWJGL version 3.3.3"));
        assert!(!d.observe("[12:00:01] [main/INFO]: Loading for game Minecraft 1.20.1"));
    }

    #[test]
    fn sound_engine_line_is_ready_once() {
        let mut d = ReadinessDetector::new();
        assert!(d.observe("[12:00:05] [Render thread/INFO]: Sound engine started"));
        assert!(!d.observe("[12:00:05] [Render thread/INFO]: Sound engine started"));
    }

    #[test]
    fn atlas_line_is_ready() {
        let mut d = ReadinessDetector::new();
        assert!(d.observe(
            "[12:00:04] [Render thread/INFO]: Created: 1024x512x0 minecraft:textures/atlas/blocks.png-atlas"
        ));
    }

    #[test]
    fn non_render_thread_quotes_do_not_count() {
        let mut d = ReadinessDetector::new();
        assert!(!d.observe("[12:00:05] [Server thread/INFO]: <Steve> Sound engine started"));
        assert!(!d.observe("Created: a new world -atlas"));
    }

    #[test]
    fn works_for_loader_prefixed_output() {
        let mut d = ReadinessDetector::new();
        assert!(d.observe("[12:00:05] [Render thread/INFO] [net.minecraft.client.sounds.SoundEngine/]: Sound engine started"));
    }
}
