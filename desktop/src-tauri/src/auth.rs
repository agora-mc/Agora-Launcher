use crate::error::{LauncherError, LauncherResult};

pub use agora_core::auth::{
    get_github_user, poll_device_flow, start_device_flow, DeviceFlowResponse, GitHubTokenBundle,
    GithubProfile, AGORA_OAUTH_CLIENT_ID,
};

pub fn log_line(line: &str) {
    eprintln!("[auth] {line}");
}

pub fn store_token(_app: &tauri::AppHandle, token: &str) -> LauncherResult<()> {
    agora_core::auth::store_token(token)
}

pub fn store_token_bundle(
    _app: &tauri::AppHandle,
    bundle: &GitHubTokenBundle,
) -> LauncherResult<()> {
    agora_core::auth::store_token_bundle(bundle)
}

pub fn get_token<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> Option<String> {
    agora_core::auth::get_token()
}

pub async fn get_valid_access_token<R: tauri::Runtime>(
    _app: &tauri::AppHandle<R>,
) -> Option<String> {
    agora_core::auth::get_valid_access_token().await
}

pub fn clear_token<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> Result<(), String> {
    agora_core::auth::clear_token()
}

pub fn is_authenticated<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> bool {
    agora_core::auth::is_authenticated()
}

/// Fetch the GitHub profile for the stored token. If the token has expired
/// (GitHub returns 401), attempt a refresh before clearing.
pub async fn get_validated_github_profile<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> LauncherResult<GithubProfile> {
    let token = get_valid_access_token(app)
        .await
        .ok_or(LauncherError::AuthRequired)?;
    match get_github_user(&token).await {
        Ok(profile) => Ok(profile),
        Err(LauncherError::AuthExpired) => {
            if agora_core::auth::try_refresh_after_401_with_token(&token)
                .await
                .is_ok()
            {
                if let Some(new_token) = get_valid_access_token(app).await {
                    return match get_github_user(&new_token).await {
                        Ok(profile) => Ok(profile),
                        Err(LauncherError::AuthExpired) => {
                            let _ = clear_token(app);
                            Err(LauncherError::AuthExpired)
                        }
                        Err(e) => Err(e),
                    };
                }
            }
            let _ = clear_token(app);
            Err(LauncherError::AuthExpired)
        }
        Err(e) => Err(e),
    }
}
