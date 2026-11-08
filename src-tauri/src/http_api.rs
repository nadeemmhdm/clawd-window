use crate::file_eater::eat_paths;
use crate::state::{ExternalUntil, PetState, SharedState, StatePayload};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use parking_lot::Mutex;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::time::sleep;

const EXTERNAL_HOLD_SECS: u64 = 30;
const DEBOUNCE_MS: u64 = 250;
const MAX_EAT_ITEMS: usize = 32;

#[derive(Clone)]
struct AppState {
    app: AppHandle,
    state: SharedState,
    external_until: ExternalUntil,
    last_external_set: Arc<Mutex<(PetState, Instant)>>,
    api_token: Arc<String>,
    allow_eat_api: bool,
}

#[derive(Deserialize)]
struct StateReq {
    state: String,
}

#[derive(Deserialize)]
struct EatReq {
    paths: Vec<String>,
}

#[derive(Serialize)]
struct EatResp {
    eaten: usize,
}

#[derive(Serialize)]
struct OkResp {
    ok: bool,
}

#[derive(Serialize)]
struct ErrResp {
    error: String,
}

fn is_authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            value
                .strip_prefix("Bearer ")
                .is_some_and(|token| token == expected)
        })
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrResp {
            error: "authorization required".into(),
        }),
    )
        .into_response()
}

async fn health(State(s): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, &s.api_token) {
        return unauthorized();
    }
    Json(OkResp { ok: true }).into_response()
}

async fn status(State(s): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, &s.api_token) {
        return unauthorized();
    }
    let cur = *s.state.read();
    Json(StatePayload::from(cur)).into_response()
}

async fn set_state_ep(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<StateReq>,
) -> Response {
    if !is_authorized(&headers, &s.api_token) {
        return unauthorized();
    }
    let next = match PetState::from_key(&req.state) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrResp {
                    error: format!("unknown state: {}", req.state),
                }),
            )
                .into_response()
        }
    };
    {
        let mut last = s.last_external_set.lock();
        if last.0 == next && last.1.elapsed() < Duration::from_millis(DEBOUNCE_MS) {
            return (StatusCode::OK, Json(OkResp { ok: true })).into_response();
        }
        *last = (next, Instant::now());
    }
    *s.external_until.lock() = Instant::now() + Duration::from_secs(EXTERNAL_HOLD_SECS);
    {
        let mut current = s.state.write();
        if *current == next {
            return (StatusCode::OK, Json(OkResp { ok: true })).into_response();
        }
        *current = next;
    }
    let payload: StatePayload = next.into();
    if let Err(e) = s.app.emit("state-changed", payload) {
        log::warn!("emit state-changed failed: {}", e);
    }
    (StatusCode::OK, Json(OkResp { ok: true })).into_response()
}

async fn celebrate(State(s): State<AppState>, headers: HeaderMap) -> Response {
    if !is_authorized(&headers, &s.api_token) {
        return unauthorized();
    }
    *s.external_until.lock() = Instant::now() + Duration::from_secs(EXTERNAL_HOLD_SECS);
    let app = s.app.clone();
    let state = s.state.clone();
    tauri::async_runtime::spawn(async move {
        write_state(&app, &state, PetState::Happy);
        sleep(Duration::from_millis(1500)).await;
        write_state(&app, &state, PetState::IdleLiving);
    });
    Json(OkResp { ok: true }).into_response()
}

async fn eat(State(s): State<AppState>, headers: HeaderMap, Json(req): Json<EatReq>) -> Response {
    if !is_authorized(&headers, &s.api_token) {
        return unauthorized();
    }
    if !s.allow_eat_api {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrResp {
                error:
                    "file eating through the API is disabled; enable CLAWD_ENABLE_EAT_API to opt in"
                        .into(),
            }),
        )
            .into_response();
    }
    if req.paths.is_empty()
        || req.paths.len() > MAX_EAT_ITEMS
        || req.paths.iter().any(|p| p.len() > 4_096)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrResp {
                error: format!("paths must contain 1–{MAX_EAT_ITEMS} short paths"),
            }),
        )
            .into_response();
    }
    let bufs: Vec<PathBuf> = req.paths.into_iter().map(PathBuf::from).collect();
    *s.external_until.lock() = Instant::now() + Duration::from_secs(EXTERNAL_HOLD_SECS);
    match eat_paths(s.app.clone(), s.state.clone(), bufs).await {
        Ok(n) => (StatusCode::OK, Json(EatResp { eaten: n })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrResp { error: e }),
        )
            .into_response(),
    }
}

fn write_state(app: &AppHandle, state: &SharedState, next: PetState) {
    {
        let mut current = state.write();
        if *current == next {
            return;
        }
        *current = next;
    }
    let payload: StatePayload = next.into();
    let _ = app.emit("state-changed", payload);
}

fn open_token_file(path: &Path, create_new: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(!create_new).write(create_new);
    if create_new {
        options.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
        if create_new {
            options.mode(0o600);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

#[cfg(windows)]
fn ensure_app_data_dir(path: &Path) -> Result<(), String> {
    let mut components: Vec<PathBuf> = path.ancestors().map(Path::to_path_buf).collect();
    components.reverse();
    for component in components {
        match std::fs::symlink_metadata(&component) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) || !metadata.is_dir() {
                    return Err(
                        "API credential directory chain contains a reparse point or non-directory"
                            .into(),
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Err(error) = std::fs::create_dir(&component) {
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(format!(
                            "could not create API credential directory: {error}"
                        ));
                    }
                }
                let metadata = std::fs::symlink_metadata(&component)
                    .map_err(|e| format!("could not inspect API credential directory: {e}"))?;
                if is_reparse_point(&metadata) || !metadata.is_dir() {
                    return Err(
                        "API credential directory chain contains a reparse point or non-directory"
                            .into(),
                    );
                }
            }
            Err(error) => {
                return Err(format!(
                    "could not inspect API credential directory: {error}"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_app_data_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|e| format!("could not create API credential directory: {e}"))
}

fn verify_app_data_dir(path: &Path, expected_canonical: &Path) -> Result<(), String> {
    ensure_app_data_dir(path)?;
    let actual = std::fs::canonicalize(path)
        .map_err(|e| format!("could not resolve API credential directory: {e}"))?;
    if actual != expected_canonical {
        return Err("API credential directory changed during setup".into());
    }
    Ok(())
}

fn valid_api_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_existing_api_token(
    path: &Path,
    app_data_dir: &Path,
    expected_canonical_dir: &Path,
) -> Result<Option<String>, String> {
    verify_app_data_dir(app_data_dir, expected_canonical_dir)?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not inspect local API credential: {error}")),
    };
    if is_reparse_point(&metadata) || !metadata.is_file() {
        return Err("local API credential must be a regular file, not a reparse point".into());
    }
    let file = open_token_file(path, false)
        .map_err(|e| format!("could not open local API credential safely: {e}"))?;
    verify_app_data_dir(app_data_dir, expected_canonical_dir)?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("could not inspect local API credential: {e}"))?;
    if is_reparse_point(&metadata) || !metadata.is_file() {
        return Err("local API credential must be a regular file, not a reparse point".into());
    }
    verify_app_data_dir(app_data_dir, expected_canonical_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("could not protect local API credential: {e}"))?;
    }
    let mut contents = Vec::new();
    file.take(513)
        .read_to_end(&mut contents)
        .map_err(|e| format!("could not read local API credential: {e}"))?;
    if contents.len() > 512 {
        return Err("local API credential file is unexpectedly large".into());
    }
    let token = String::from_utf8(contents)
        .map_err(|_| "local API credential is not valid text".to_string())?;
    let token = token.trim();
    if valid_api_token(token) {
        Ok(Some(token.to_string()))
    } else {
        Err("local API credential file contains an invalid token".into())
    }
}

fn load_or_create_api_token(app: &AppHandle) -> Result<String, String> {
    if let Some(token) = std::env::var_os("CLAWD_API_TOKEN") {
        let token = token
            .to_str()
            .ok_or_else(|| "CLAWD_API_TOKEN must be valid text".to_string())?;
        if token.len() < 32 || token.len() > 512 || token.chars().any(char::is_control) {
            return Err("CLAWD_API_TOKEN must be 32–512 non-control characters".into());
        }
        return Ok(token.to_string());
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data directory unavailable: {e}"))?;
    ensure_app_data_dir(&dir)?;
    let canonical_dir = std::fs::canonicalize(&dir)
        .map_err(|e| format!("could not resolve API credential directory: {e}"))?;
    verify_app_data_dir(&dir, &canonical_dir)?;
    let dir_metadata = std::fs::symlink_metadata(&dir)
        .map_err(|e| format!("could not inspect API credential directory: {e}"))?;
    if is_reparse_point(&dir_metadata) || !dir_metadata.is_dir() {
        return Err("API credential directory must not be a reparse point".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("could not protect API credential directory: {e}"))?;
    }
    let path = dir.join("clawd-api-token");
    verify_app_data_dir(&dir, &canonical_dir)?;
    if let Some(token) = read_existing_api_token(&path, &dir, &canonical_dir)? {
        return Ok(token);
    }
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    verify_app_data_dir(&dir, &canonical_dir)?;
    match open_token_file(&path, true) {
        Ok(mut file) => {
            if let Err(error) = verify_app_data_dir(&dir, &canonical_dir) {
                return Err(error);
            }
            let metadata = file
                .metadata()
                .map_err(|e| format!("could not inspect new local API credential: {e}"))?;
            if is_reparse_point(&metadata) || !metadata.is_file() {
                return Err("new local API credential is not a regular file".into());
            }
            if let Err(error) = file.write_all(token.as_bytes()) {
                return Err(format!("could not save local API credential: {error}"));
            }
            file.sync_all()
                .map_err(|e| format!("could not flush local API credential: {e}"))?;
            Ok(token)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            read_existing_api_token(&path, &dir, &canonical_dir)?
                .ok_or_else(|| "local API credential changed during setup".to_string())
        }
        Err(error) => Err(format!(
            "could not create local API credential safely: {error}"
        )),
    }
}

fn api_eat_enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

pub fn spawn_http_server(app: AppHandle, state: SharedState, external_until: ExternalUntil) {
    let api_token = match load_or_create_api_token(&app) {
        Ok(token) => Arc::new(token),
        Err(error) => {
            log::error!("local API disabled: {}", error);
            return;
        }
    };
    let port: u16 = std::env::var("CLAWD_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9876);
    let app_state = AppState {
        app,
        state,
        external_until,
        last_external_set: Arc::new(Mutex::new((
            PetState::IdleLiving,
            Instant::now() - Duration::from_secs(60),
        ))),
        api_token,
        allow_eat_api: api_eat_enabled(std::env::var("CLAWD_ENABLE_EAT_API").ok().as_deref()),
    };
    let router = Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/state", post(set_state_ep))
        .route("/celebrate", post(celebrate))
        .route("/eat", post(eat))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(app_state);
    tauri::async_runtime::spawn(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                log::info!("authenticated HTTP API listening on {}", addr);
                if let Err(e) = axum::serve(listener, router).await {
                    log::warn!("HTTP API stopped: {}", e);
                }
            }
            Err(e) => log::warn!(
                "HTTP API bind failed on {}: {} (continuing without API)",
                addr,
                e
            ),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn local_api_requires_exact_bearer_token() {
        let expected = "test-token-expected";
        let mut headers = HeaderMap::new();
        assert!(!is_authorized(&headers, expected));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong-token"),
        );
        assert!(!is_authorized(&headers, expected));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer test-token-expected"),
        );
        assert!(is_authorized(&headers, expected));
    }

    #[test]
    fn api_file_eating_requires_deliberate_opt_in() {
        assert!(!api_eat_enabled(None));
        assert!(!api_eat_enabled(Some("false")));
        assert!(api_eat_enabled(Some("true")));
        assert!(api_eat_enabled(Some("1")));
    }

    #[cfg(unix)]
    #[test]
    fn token_file_creation_is_exclusive_and_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("clawd-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let canonical_dir = std::fs::canonicalize(&dir).unwrap();
        let path = dir.join("token");
        let mut file = open_token_file(&path, true).unwrap();
        file.write_all(b"a".repeat(64).as_slice()).unwrap();
        drop(file);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
        assert!(open_token_file(&path, true).is_err());
        assert_eq!(
            read_existing_api_token(&path, &dir, &canonical_dir).unwrap(),
            Some("a".repeat(64))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn token_file_reader_rejects_symlinks() {
        let dir = std::env::temp_dir().join(format!("clawd-token-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let canonical_dir = std::fs::canonicalize(&dir).unwrap();
        let target = dir.join("target");
        let link = dir.join("token");
        std::fs::write(&target, "a".repeat(64)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_existing_api_token(&link, &dir, &canonical_dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn token_directory_replacement_is_rejected() {
        let base = std::env::temp_dir().join(format!("clawd-token-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("app-data");
        let outside = base.join("outside");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let canonical_dir = std::fs::canonicalize(&dir).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&outside, &dir).unwrap();
        assert!(verify_app_data_dir(&dir, &canonical_dir).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
