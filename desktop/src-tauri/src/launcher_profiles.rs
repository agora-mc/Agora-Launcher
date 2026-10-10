use crate::error::{LauncherError, LauncherResult};

pub use agora_game_minecraft::launcher_profiles::LauncherProfileEntry;

pub fn upsert_profile(entry: &LauncherProfileEntry) -> LauncherResult<()> {
    let profiles_path =
        crate::paths::launcher_profiles_path().ok_or(LauncherError::MojangNotFound)?;
    agora_game_minecraft::launcher_profiles::upsert_profile(entry, &profiles_path)
}

pub fn remove_profile(profile_id: &str) -> LauncherResult<()> {
    let profiles_path =
        crate::paths::launcher_profiles_path().ok_or(LauncherError::MojangNotFound)?;
    agora_game_minecraft::launcher_profiles::remove_profile(profile_id, &profiles_path)
}
