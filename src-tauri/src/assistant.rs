use chrono::{DateTime, Local};
use keyring::Entry;
use parking_lot::Mutex;
use rand::{rngs::OsRng, RngCore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::time::{interval, MissedTickBehavior};

const KEYRING_SERVICE: &str = "com.clawd.pet";
const KEYRING_USER: &str = "ollama-cloud-api-key";
const DEFAULT_MODEL: &str = "gemma4:31b";
const MAX_PROMPT_CHARS: usize = 2_000;
const MAX_TITLE_CHARS: usize = 200;
const MAX_RESULTS: usize = 40;
const MAX_VISITED_ENTRIES: usize = 12_000;
const MAX_SEARCH_DEPTH: usize = 6;
const SEARCH_STOP_WORDS: &[&str] = &[
    "find",
    "search",
    "locate",
    "look",
    "please",
    "where",
    "show",
    "open",
    "get",
    "the",
    "a",
    "an",
    "my",
    "me",
    "for",
    "in",
    "on",
    "from",
    "of",
    "to",
    "with",
    "and",
    "can",
    "you",
    "file",
    "files",
    "document",
    "documents",
    "folder",
    "folders",
    "download",
    "downloads",
    "pdf",
    "kandethan",
    "kandeth",
    "kandu",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub due_at_ms: Option<i64>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub reminder_notified: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedData {
    #[serde(default = "default_model")]
    model: String,
    #[serde(default)]
    enabled_folders: Vec<String>,
    #[serde(default)]
    todos: Vec<TodoItem>,
}

impl Default for PersistedData {
    fn default() -> Self {
        Self {
            model: default_model(),
            enabled_folders: Vec::new(),
            todos: Vec::new(),
        }
    }
}

fn default_model() -> String {
    DEFAULT_MODEL.to_string()
}

#[derive(Clone)]
pub struct AssistantState {
    file_path: PathBuf,
    data: Arc<Mutex<PersistedData>>,
    persist_lock: Arc<Mutex<()>>,
}

impl AssistantState {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let file_path = app_data_dir.join("assistant-data.json");
        let data = std::fs::read(&file_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PersistedData>(&bytes).ok())
            .unwrap_or_default();
        Self {
            file_path,
            data: Arc::new(Mutex::new(data)),
            persist_lock: Arc::new(Mutex::new(())),
        }
    }

    fn persist(&self) -> Result<(), String> {
        let data = self.data.lock().clone();
        let _write_guard = self.persist_lock.lock();
        persist_data(&self.file_path, &data)
    }
}

fn persist_data(path: &Path, data: &PersistedData) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "invalid assistant data path".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("could not create app data folder: {e}"))?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(data)
        .map_err(|e| format!("could not encode assistant data: {e}"))?;
    std::fs::write(&temp, bytes).map_err(|e| format!("could not save assistant data: {e}"))?;
    std::fs::rename(&temp, path).map_err(|e| format!("could not update assistant data: {e}"))
}

#[derive(Clone, Debug, Serialize)]
pub struct FolderOption {
    pub id: String,
    pub name: String,
    pub path: String,
    pub exists: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssistantSettings {
    pub model: String,
    pub api_key_configured: bool,
    pub api_key_status: String,
    pub api_key_issue: Option<String>,
    pub folders: Vec<FolderOption>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileMatch {
    pub name: String,
    pub folder_name: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ChatReply {
    pub text: String,
    pub files: Vec<FileMatch>,
    pub todos: Vec<TodoItem>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug)]
enum ApiKeyReadError {
    Missing,
    Invalid(String),
    StoreUnavailable(String),
}

impl ApiKeyReadError {
    fn user_message(&self) -> String {
        match self {
            Self::Missing => {
                "Add your Ollama Cloud API key in Clawd Settings to start chatting.".to_string()
            }
            Self::Invalid(message) => message.clone(),
            Self::StoreUnavailable(message) => message.clone(),
        }
    }
}

fn api_key_status(result: Result<String, ApiKeyReadError>) -> (bool, &'static str, Option<String>) {
    match result {
        Ok(_) => (true, "saved", None),
        Err(ApiKeyReadError::Missing) => (false, "missing", None),
        Err(ApiKeyReadError::Invalid(message)) => (false, "invalid", Some(message)),
        Err(ApiKeyReadError::StoreUnavailable(message)) => (false, "unavailable", Some(message)),
    }
}

fn keyring_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("system credential store unavailable: {e}"))
}

fn normalize_api_key(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let key = match raw.split_once(' ') {
        Some((scheme, value)) if scheme.eq_ignore_ascii_case("bearer") => value.trim(),
        _ => raw,
    };
    if key.len() < 8 || key.len() > 512 || key.chars().any(char::is_control) {
        return Err(
            "The API key must be between 8 and 512 characters, without control characters.".into(),
        );
    }
    Ok(key.to_string())
}

fn read_api_key() -> Result<String, ApiKeyReadError> {
    let entry = keyring_entry().map_err(ApiKeyReadError::StoreUnavailable)?;
    let raw = match entry.get_password() {
        Ok(key) => key,
        Err(keyring::Error::NoEntry) => return Err(ApiKeyReadError::Missing),
        Err(error) => {
            return Err(ApiKeyReadError::StoreUnavailable(format!(
                "Could not read the saved key from the system credential store: {error}"
            )))
        }
    };
    if raw.trim().is_empty() {
        return Err(ApiKeyReadError::Missing);
    }
    normalize_api_key(&raw).map_err(|_| {
        ApiKeyReadError::Invalid(
            "The saved API key is invalid. Re-enter it in Clawd Settings.".into(),
        )
    })
}

#[tauri::command]
pub fn assistant_settings(state: State<'_, AssistantState>) -> AssistantSettings {
    let data = state.data.lock();
    let (configured, key_status, issue) = api_key_status(read_api_key());
    let folders = folder_options(&data.enabled_folders);
    AssistantSettings {
        model: data.model.clone(),
        api_key_configured: configured,
        api_key_status: key_status.to_string(),
        api_key_issue: issue,
        folders,
    }
}

#[tauri::command]
pub fn save_ollama_api_key(api_key: String) -> Result<(), String> {
    let key = normalize_api_key(&api_key)?;
    let entry = keyring_entry()?;
    entry
        .set_password(&key)
        .map_err(|e| format!("could not save key in the system credential store: {e}"))?;
    let saved = entry.get_password().map_err(|e| {
        format!("the key was written but could not be verified in the system credential store: {e}")
    })?;
    let saved = normalize_api_key(&saved).map_err(|_| {
        "The saved API key could not be verified. Please try saving it again.".to_string()
    })?;
    if saved != key {
        return Err("The saved API key did not match the value entered. Please try again.".into());
    }
    Ok(())
}

#[tauri::command]
pub fn clear_ollama_api_key() -> Result<(), String> {
    let entry = keyring_entry()?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(format!("could not clear the saved key: {error}")),
    }
    match entry.get_password() {
        Err(keyring::Error::NoEntry) => Ok(()),
        Ok(_) => Err("the credential store still contains an entry after clearing the key".into()),
        Err(error) => Err(format!(
            "could not verify that the saved key was cleared: {error}"
        )),
    }
}

#[tauri::command]
pub fn set_ollama_model(model: String, state: State<'_, AssistantState>) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty()
        || model.len() > 100
        || !model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":-_/.".contains(c))
    {
        return Err("Use a model name containing letters, numbers, dots, dashes, underscores, slashes or colons.".into());
    }
    state.data.lock().model = model.to_string();
    state.persist()
}

#[tauri::command]
pub fn set_enabled_folders(
    folder_ids: Vec<String>,
    state: State<'_, AssistantState>,
) -> Result<(), String> {
    if folder_ids.len() > 5 {
        return Err("Choose from the five supported folders.".into());
    }
    let allowed = ["downloads", "documents", "pictures", "videos", "music"];
    let mut unique = Vec::new();
    for id in folder_ids {
        if !allowed.contains(&id.as_str()) {
            return Err("Unknown folder permission.".into());
        }
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    state.data.lock().enabled_folders = unique;
    state.persist()
}

#[tauri::command]
pub fn list_todos(state: State<'_, AssistantState>) -> Vec<TodoItem> {
    let mut todos = state.data.lock().todos.clone();
    todos.sort_by_key(|t| {
        (
            t.completed,
            t.due_at_ms.unwrap_or(i64::MAX),
            t.created_at_ms,
        )
    });
    todos
}

#[tauri::command]
pub fn add_todo(title: String, state: State<'_, AssistantState>) -> Result<TodoItem, String> {
    create_todo(&state, title, None)
}

#[tauri::command]
pub fn complete_todo(
    id: String,
    completed: bool,
    state: State<'_, AssistantState>,
) -> Result<(), String> {
    let mut data = state.data.lock();
    let todo = data
        .todos
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or_else(|| "task not found".to_string())?;
    todo.completed = completed;
    if completed {
        todo.reminder_notified = true;
    }
    drop(data);
    state.persist()
}

#[tauri::command]
pub fn delete_todo(id: String, state: State<'_, AssistantState>) -> Result<(), String> {
    let mut data = state.data.lock();
    let old_len = data.todos.len();
    data.todos.retain(|t| t.id != id);
    if data.todos.len() == old_len {
        return Err("task not found".into());
    }
    drop(data);
    state.persist()
}

fn create_todo(
    state: &AssistantState,
    title: String,
    due_at_ms: Option<i64>,
) -> Result<TodoItem, String> {
    let title = title.trim();
    if title.is_empty()
        || title.chars().count() > MAX_TITLE_CHARS
        || title.chars().any(char::is_control)
    {
        return Err(format!(
            "Task/reminder text must be 1–{MAX_TITLE_CHARS} printable characters."
        ));
    }
    if let Some(due) = due_at_ms {
        let now = chrono::Utc::now().timestamp_millis();
        if due <= now || due > now + 10 * 366 * 24 * 60 * 60 * 1000_i64 {
            return Err("A reminder time must be in the future and within ten years.".into());
        }
    }
    let todo = TodoItem {
        id: random_id(),
        title: title.to_string(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
        due_at_ms,
        completed: false,
        reminder_notified: false,
    };
    state.data.lock().todos.push(todo.clone());
    state.persist()?;
    Ok(todo)
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn folder_paths() -> Vec<(String, String, PathBuf)> {
    let mut folders = Vec::new();
    if let Some(path) = dirs::download_dir() {
        folders.push(("downloads".into(), "Downloads".into(), path));
    }
    if let Some(path) = dirs::document_dir() {
        folders.push(("documents".into(), "Documents".into(), path));
    }
    if let Some(path) = dirs::picture_dir() {
        folders.push(("pictures".into(), "Pictures".into(), path));
    }
    if let Some(path) = dirs::video_dir() {
        folders.push(("videos".into(), "Videos".into(), path));
    }
    if let Some(path) = dirs::audio_dir() {
        folders.push(("music".into(), "Music".into(), path));
    }
    folders
}

fn folder_options(enabled: &[String]) -> Vec<FolderOption> {
    folder_paths()
        .into_iter()
        .map(|(id, name, path)| FolderOption {
            enabled: enabled.contains(&id),
            exists: validated_known_folder_root(&path).is_some(),
            path: path.to_string_lossy().into_owned(),
            id,
            name,
        })
        .collect()
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

fn known_folder_has_reparse_component(path: &Path) -> bool {
    #[cfg(windows)]
    {
        for component in path.ancestors() {
            if std::fs::symlink_metadata(component)
                .map(|metadata| is_reparse_point(&metadata))
                .unwrap_or(true)
            {
                return true;
            }
        }
        false
    }
    #[cfg(not(windows))]
    {
        std::fs::symlink_metadata(path)
            .map(|metadata| is_reparse_point(&metadata))
            .unwrap_or(true)
    }
}

fn validated_known_folder_root(path: &Path) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if known_folder_has_reparse_component(path) || !metadata.is_dir() {
        return None;
    }
    let canonical = std::fs::canonicalize(path).ok()?;
    if known_folder_has_reparse_component(path) || std::fs::canonicalize(path).ok()? != canonical {
        return None;
    }
    std::fs::metadata(&canonical)
        .ok()?
        .is_dir()
        .then_some(canonical)
}

fn enabled_roots(state: &AssistantState) -> Vec<(String, String, PathBuf)> {
    let enabled = state.data.lock().enabled_folders.clone();
    folder_paths()
        .into_iter()
        .filter_map(|(id, name, path)| {
            if !enabled.contains(&id) {
                return None;
            }
            validated_known_folder_root(&path).map(|root| (id, name, root))
        })
        .collect()
}

#[tauri::command]
pub async fn search_files(
    query: String,
    state: State<'_, AssistantState>,
) -> Result<Vec<FileMatch>, String> {
    let query = query.trim().to_lowercase();
    if query.chars().count() < 2
        || query.chars().count() > 120
        || query.chars().any(char::is_control)
    {
        return Err("Search text must be between 2 and 120 characters.".into());
    }
    if state.data.lock().enabled_folders.is_empty() {
        return Err(
            "No folders are enabled. Turn on filename search for one or more folders in Settings."
                .into(),
        );
    }
    let roots = enabled_roots(&state);
    if roots.is_empty() {
        return Err(
            "The folders you enabled are not available right now. Check their locations in Settings."
                .into(),
        );
    }
    tauri::async_runtime::spawn_blocking(move || search_roots(&query, roots))
        .await
        .map_err(|e| format!("file search failed: {e}"))?
}

fn normalized_search_words(value: &str) -> Vec<String> {
    value
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

fn filename_matches_query(name: &str, query: &str) -> bool {
    let query_words = normalized_search_words(query);
    let wants_pdf = query_words.iter().any(|word| word == "pdf");
    if wants_pdf
        && !Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
    {
        return false;
    }

    let meaningful_words: Vec<&str> = query_words
        .iter()
        .map(String::as_str)
        .filter(|word| !SEARCH_STOP_WORDS.contains(word))
        .collect();
    if meaningful_words.is_empty() {
        return wants_pdf;
    }

    let name_words = normalized_search_words(name);
    let normalized_name = name_words.join(" ");
    let compact_name = name_words.join("");
    let compact_query = query_words.join("");
    if normalized_name.contains(&query_words.join(" ")) || compact_name.contains(&compact_query) {
        return true;
    }

    let matches = meaningful_words
        .iter()
        .filter(|word| normalized_name.contains(**word) || compact_name.contains(**word))
        .count();
    let required = if meaningful_words.len() <= 2 {
        meaningful_words.len()
    } else {
        2
    };
    matches >= required
}

fn search_roots(
    query: &str,
    roots: Vec<(String, String, PathBuf)>,
) -> Result<Vec<FileMatch>, String> {
    let mut results = Vec::new();
    let mut visited = 0usize;
    for (_id, folder_name, root) in roots {
        // `enabled_roots` already returns canonical roots. Do not replace that
        // trusted boundary with a new canonical target: if a junction appeared
        // after validation, it must invalidate this root, not redefine it.
        match std::fs::canonicalize(&root) {
            Ok(canonical) if canonical == root => {}
            Err(_) => continue,
            _ => continue,
        }
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(v) => v,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                visited += 1;
                if visited > MAX_VISITED_ENTRIES || results.len() >= MAX_RESULTS {
                    return Ok(results);
                }
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let metadata = match std::fs::symlink_metadata(&path) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if is_reparse_point(&metadata) {
                    continue;
                }
                let canonical = match std::fs::canonicalize(&path) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if !is_within_root(&canonical, &root) {
                    continue;
                }
                if metadata.is_dir() {
                    if depth < MAX_SEARCH_DEPTH {
                        stack.push((canonical, depth + 1));
                    }
                } else if metadata.is_file() && filename_matches_query(&name, query) {
                    results.push(FileMatch {
                        name,
                        folder_name: folder_name.clone(),
                        path: canonical.to_string_lossy().into_owned(),
                    });
                    if results.len() >= MAX_RESULTS {
                        return Ok(results);
                    }
                }
            }
        }
    }
    Ok(results)
}

fn is_within_root(candidate: &Path, root: &Path) -> bool {
    candidate != root && candidate.starts_with(root)
}

fn validated_file_path(path: &str, state: &AssistantState) -> Result<PathBuf, String> {
    if path.len() > 4_096 {
        return Err("Invalid file location.".into());
    }
    let candidate =
        std::fs::canonicalize(path).map_err(|_| "File is no longer available.".to_string())?;
    if !candidate.is_file() {
        return Err("Only files can be revealed.".into());
    }
    for (_, _, allowed_root) in enabled_roots(state) {
        // `enabled_roots` returns the canonical, validated root. Keep that
        // boundary fixed; re-canonicalizing it here could bless a replacement
        // junction as a new root for the reveal check.
        if is_within_root(&candidate, &allowed_root) {
            return Ok(candidate);
        }
    }
    Err("That file is outside the folders you enabled.".into())
}

#[tauri::command]
pub fn reveal_file(
    path: String,
    app: AppHandle,
    state: State<'_, AssistantState>,
) -> Result<(), String> {
    let path = validated_file_path(&path, &state)?;
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.to_string_lossy()))
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(Path::new(".")))
        .spawn();
    #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
    let result: std::io::Result<std::process::Child> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "unsupported platform",
    ));
    result
        .map(|_| ())
        .map_err(|e| format!("could not open the file location: {e}"))?;
    if let Some(win) = app.get_webview_window("assistant") {
        let _ = win.set_focus();
    }
    Ok(())
}

fn make_tools() -> Value {
    json!([
      {"type":"function","function":{"name":"add_task","description":"Add a concrete future action to Clawd's local task list when the user asks for a task or clearly says they need, plan, or intend to do something, or asks you to remember it. Do not require the literal words task or TODO.","parameters":{"type":"object","properties":{"title":{"type":"string"}},"required":["title"]}}},
      {"type":"function","function":{"name":"set_reminder","description":"Create a local reminder only when the user explicitly asks to be reminded. Use an RFC3339 due_at timestamp with the user's local UTC offset.","parameters":{"type":"object","properties":{"title":{"type":"string"},"due_at":{"type":"string","description":"RFC3339 local time with UTC offset, e.g. 2026-11-08T17:30:00+05:30"}},"required":["title","due_at"]}}},
      {"type":"function","function":{"name":"search_files","description":"Search filenames only inside folders the user enabled. Never read file contents. Match important words across spaces, hyphens, underscores, or combined words; treat PDF as a file-type filter. Use a short query when the user asks to find or locate a file.","parameters":{"type":"object","properties":{"query":{"type":"string","description":"Short identifying filename words or file type"}},"required":["query"]}}}
    ])
}

fn provider_error_hint(body: &[u8], api_key: &str) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let raw = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.get("message").and_then(Value::as_str))?;
    let sanitized = raw
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .replace(api_key, "[redacted]");
    let short = sanitized.chars().take(240).collect::<String>();
    let short = short.trim();
    (!short.is_empty()).then(|| short.to_string())
}

#[cfg(test)]
mod api_key_and_error_tests {
    use super::*;

    #[test]
    fn normalize_api_key_accepts_raw_and_bearer_values() {
        assert_eq!(normalize_api_key("  abcdefgh  ").unwrap(), "abcdefgh");
        assert_eq!(normalize_api_key("Bearer abcdefgh").unwrap(), "abcdefgh");
        assert_eq!(normalize_api_key("bEaReR abcdefgh").unwrap(), "abcdefgh");
    }

    #[test]
    fn normalize_api_key_rejects_control_characters() {
        assert!(normalize_api_key("abcdefgh\nijklmnop").is_err());
    }

    #[test]
    fn invalid_key_and_unavailable_store_have_distinct_statuses() {
        let invalid = api_key_status(Err(ApiKeyReadError::Invalid("invalid format".into())));
        let unavailable = api_key_status(Err(ApiKeyReadError::StoreUnavailable(
            "store unavailable".into(),
        )));
        assert_eq!(invalid, (false, "invalid", Some("invalid format".into())));
        assert_eq!(
            unavailable,
            (false, "unavailable", Some("store unavailable".into()))
        );
    }

    #[test]
    fn provider_error_hint_redacts_the_api_key() {
        let key = "secret-key-123456";
        let body = format!(r#"{{"error":{{"message":"Invalid key {key}"}}}}"#);
        assert_eq!(
            provider_error_hint(body.as_bytes(), key).as_deref(),
            Some("Invalid key [redacted]")
        );
    }
}

fn local_clock_context() -> String {
    format!("Current local system time is {}. Use this local timezone when interpreting reminder times; ask a brief clarifying question if the date or time is ambiguous.", Local::now().to_rfc3339())
}

#[tauri::command]
pub async fn assistant_chat(
    prompt: String,
    history: Vec<ChatMessage>,
    state: State<'_, AssistantState>,
) -> Result<ChatReply, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(format!("Message must be 1–{MAX_PROMPT_CHARS} characters."));
    }
    let api_key = read_api_key().map_err(|error| error.user_message())?;
    let model = state.data.lock().model.clone();
    let mut messages = vec![json!({"role":"system","content":format!(
        "You are Clawd, a friendly and concise desktop pet assistant. Be warm, reassuring, and honest; reply in the user's language. {} You may use only these tools: add a task, create a reminder, or search filenames in folders the user has explicitly enabled. Automatically add a task when the user clearly describes a concrete future action they need or plan to do, or asks you to remember it; do not make them say the words task or TODO. This includes casual phrases such as ‘I need to…’, ‘I have to…’, ‘don’t let me forget…’, ‘remember to…’, and equivalent Malayalam/Manglish phrases like ‘… cheyyanam’, ‘… cheyyan undu’, or ‘… marakkaruthu’. Do not add tasks for completed actions or when the user is only asking for advice. Create a reminder only when they explicitly ask to be reminded; ask a short question if the time is unclear. Never claim an action succeeded before a tool succeeds. Never read, upload, modify, delete, or execute local files; filename search returns names and locations only and does not inspect document contents. Never run commands or control other apps. For reminders, use RFC3339 with the current local offset. If another request is ambiguous, ask instead of guessing.", local_clock_context()
    )})];
    for message in history
        .into_iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        if (message.role == "user" || message.role == "assistant")
            && message.content.len() <= MAX_PROMPT_CHARS * 2
        {
            messages.push(json!({"role":message.role,"content":message.content}));
        }
    }
    messages.push(json!({"role":"user","content":prompt}));
    let request = json!({
        "model": model,
        "messages": messages,
        "tools": make_tools(),
        "tool_choice": "auto",
        "stream": false,
        "max_tokens": 800,
        "temperature": 0.5
    });
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("could not prepare Ollama request: {e}"))?;
    let response = client
        .post("https://ollama.com/v1/chat/completions")
        .bearer_auth(&api_key)
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "Ollama Cloud did not respond in time. Please try again.".to_string()
            } else if error.is_connect() {
                "Could not connect to Ollama Cloud. Check your internet connection and try again."
                    .to_string()
            } else {
                "The Ollama Cloud request failed. Check your connection and try again.".to_string()
            }
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.bytes().await.unwrap_or_default();
        let detail = provider_error_hint(&error_body, &api_key);
        return Err(match status.as_u16() {
            401 | 403 => {
                "Ollama rejected the saved API key. Check or replace it in Settings.".into()
            }
            429 => {
                "Ollama Cloud is rate limiting requests. Please wait a moment and try again.".into()
            }
            _ => match detail {
                Some(detail) => format!("Ollama Cloud returned HTTP {}: {detail}", status.as_u16()),
                None => format!(
                    "Ollama Cloud returned HTTP {}. Check the selected model and try again.",
                    status.as_u16()
                ),
            },
        });
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Could not read the Ollama response.".to_string())?;
    if bytes.len() > 1_000_000 {
        return Err("Ollama returned an unexpectedly large response.".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Ollama returned an invalid response.".to_string())?;
    let message = value
        .pointer("/choices/0/message")
        .ok_or_else(|| "Ollama returned no chat message. Try another model.".to_string())?;
    let tool_calls = message.get("tool_calls").and_then(Value::as_array);
    if let Some(call) = tool_calls.and_then(|calls| calls.first()) {
        let name = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let raw_args = call
            .pointer("/function/arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let args: Value = if let Some(text) = raw_args.as_str() {
            serde_json::from_str(text).map_err(|_| {
                "The assistant returned an invalid action. Please rephrase.".to_string()
            })?
        } else {
            raw_args
        };
        return handle_tool_call(name, &args, &state).await;
    }
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("Ollama returned an empty answer. Try again.".into());
    }
    Ok(ChatReply {
        text,
        files: Vec::new(),
        todos: Vec::new(),
    })
}

async fn handle_tool_call(
    name: &str,
    args: &Value,
    state: &AssistantState,
) -> Result<ChatReply, String> {
    match name {
        "add_task" => {
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| "I need a short task name to add that.".to_string())?;
            let todo = create_todo(state, title.to_string(), None)?;
            Ok(ChatReply {
                text: format!("Added “{}” to your task list.", todo.title),
                files: Vec::new(),
                todos: vec![todo],
            })
        }
        "set_reminder" => {
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| "I need a reminder title.".to_string())?;
            let due_text = args
                .get("due_at")
                .and_then(Value::as_str)
                .ok_or_else(|| "I need a date and time for that reminder.".to_string())?;
            let due_at = DateTime::parse_from_rfc3339(due_text)
                .map_err(|_| {
                    "I could not understand the reminder time. Please include the date and time."
                        .to_string()
                })?
                .timestamp_millis();
            let todo = create_todo(state, title.to_string(), Some(due_at))?;
            let local = DateTime::from_timestamp_millis(due_at)
                .map(|d| {
                    d.with_timezone(&Local)
                        .format("%b %-d at %-I:%M %p")
                        .to_string()
                })
                .unwrap_or_else(|| due_text.to_string());
            Ok(ChatReply {
                text: format!("I’ll remind you about “{}” on {}.", todo.title, local),
                files: Vec::new(),
                todos: vec![todo],
            })
        }
        "search_files" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .ok_or_else(|| "Tell me the filename you want to find.".to_string())?
                .to_string();
            let query = query.trim().to_lowercase();
            if query.chars().count() < 2
                || query.chars().count() > 120
                || query.chars().any(char::is_control)
            {
                return Err("Search text must be between 2 and 120 characters.".into());
            }
            if state.data.lock().enabled_folders.is_empty() {
                return Err("No folders are enabled yet. Open Settings and allow filename search in the folders you want.".into());
            }
            let roots = enabled_roots(state);
            if roots.is_empty() {
                return Err("The folders you enabled are not available right now. Check their locations in Settings.".into());
            }
            let files = tauri::async_runtime::spawn_blocking(move || search_roots(&query, roots))
                .await
                .map_err(|e| format!("file search failed: {e}"))??;
            let text = if files.is_empty() {
                "I couldn’t find a matching filename in the folders you enabled. I search names only, not document contents; try fewer words from the file’s name.".to_string()
            } else {
                format!("I found {} matching file(s). Choose “Open location” to show one in your file manager.", files.len())
            };
            Ok(ChatReply {
                text,
                files,
                todos: Vec::new(),
            })
        }
        _ => Err("That assistant action is not available.".into()),
    }
}

pub fn spawn_reminder_scheduler(app: AppHandle, state: AssistantState) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = interval(Duration::from_secs(10));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let now = chrono::Utc::now().timestamp_millis();
            let due: Vec<TodoItem> = {
                let mut data = state.data.lock();
                let mut due = Vec::new();
                for task in &mut data.todos {
                    if !task.completed
                        && !task.reminder_notified
                        && task.due_at_ms.is_some_and(|t| t <= now)
                    {
                        task.reminder_notified = true;
                        due.push(task.clone());
                    }
                }
                due
            };
            if !due.is_empty() {
                let _ = state.persist();
            }
            for task in due {
                let _ = app.emit("reminder-due", task);
                if let Some(win) = app.get_webview_window("assistant") {
                    let _ = win.show();
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_must_be_a_child_of_the_enabled_root() {
        let root = PathBuf::from("/home/test/Documents");
        assert!(is_within_root(&root.join("notes.txt"), &root));
        assert!(!is_within_root(&root, &root));
        assert!(!is_within_root(
            Path::new("/home/test/Documents-old/notes.txt"),
            &root
        ));
    }

    #[test]
    fn reminder_requires_a_future_time() {
        let state = AssistantState {
            file_path: std::env::temp_dir().join("clawd-test-assistant.json"),
            data: Arc::new(Mutex::new(PersistedData::default())),
            persist_lock: Arc::new(Mutex::new(())),
        };
        let past = chrono::Utc::now().timestamp_millis() - 1000;
        assert!(create_todo(&state, "Too late".into(), Some(past)).is_err());
        let _ = std::fs::remove_file(state.file_path);
    }

    #[test]
    fn filename_search_is_bounded_to_visible_files_under_the_root() {
        let base = std::env::temp_dir().join(format!("clawd-search-{}", random_id()));
        let root = base.join("Documents");
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(root.join("apple.png"), b"not read").unwrap();
        std::fs::write(root.join("nested/Apple-notes.txt"), b"not read").unwrap();
        std::fs::write(root.join(".private-apple.txt"), b"not read").unwrap();
        #[cfg(unix)]
        {
            let outside = base.join("outside/apple-secret.png");
            std::fs::write(&outside, b"not read").unwrap();
            std::os::unix::fs::symlink(outside, root.join("linked-apple.png")).unwrap();
        }
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let results = search_roots(
            "apple",
            vec![("documents".into(), "Documents".into(), canonical_root)],
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|r| r.name == "apple.png"));
        assert!(results.iter().any(|r| r.name == "Apple-notes.txt"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn filename_search_matches_word_variants_and_pdf_filter() {
        assert!(filename_matches_query(
            "Cyber-Security_Guide.PDF",
            "find cyber security pdf in Downloads"
        ));
        assert!(filename_matches_query(
            "Cyber-Security_Guide.PDF",
            "find cybersecurity pdf in Downloads"
        ));
        assert!(filename_matches_query(
            "Cybersecurity-Guide.pdf",
            "cyber security PDF"
        ));
        assert!(filename_matches_query(
            "Cyber Security.pdf",
            "cybersecurity"
        ));
        assert!(filename_matches_query("manual.pdf", "PDF"));
        assert!(!filename_matches_query(
            "cyber-security.txt",
            "cyber security PDF"
        ));
        assert!(!filename_matches_query(
            "accounting.pdf",
            "cyber security PDF"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn search_does_not_recanonicalize_a_replaced_root_symlink() {
        let base = std::env::temp_dir().join(format!("clawd-root-race-{}", random_id()));
        let root = base.join("Documents");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("apple-secret.png"), b"not read").unwrap();
        let trusted_root = std::fs::canonicalize(&root).unwrap();
        std::fs::remove_dir(&root).unwrap();
        std::os::unix::fs::symlink(&outside, &root).unwrap();
        let results = search_roots(
            "apple",
            vec![("documents".into(), "Documents".into(), trusted_root)],
        )
        .unwrap();
        assert!(results.is_empty());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_known_folder_root_is_rejected() {
        let base = std::env::temp_dir().join(format!("clawd-root-{}", random_id()));
        let target = base.join("target");
        let root = base.join("Documents");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &root).unwrap();
        assert!(validated_known_folder_root(&root).is_none());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn tasks_are_persisted_locally() {
        let dir = std::env::temp_dir().join(format!("clawd-data-{}", random_id()));
        let state = AssistantState::new(dir.clone());
        let task = create_todo(&state, "Buy tea".into(), None).unwrap();
        let bytes = std::fs::read(dir.join("assistant-data.json")).unwrap();
        let saved: PersistedData = serde_json::from_slice(&bytes).unwrap();
        assert!(saved
            .todos
            .iter()
            .any(|item| item.id == task.id && item.title == "Buy tea"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
