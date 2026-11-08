use crate::state::{PetState, SharedState};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::sync::oneshot;
use tokio::time::sleep;

const MAX_PATHS: usize = 32;
const MAX_DIRECT_FOLDER_ITEMS: usize = 1000;

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_REPARSE_POINT; includes junctions as well as symlinks.
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(unix)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(not(any(windows, unix)))]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    created: u64,
    written: u64,
    length: u64,
    attributes: u32,
    modified: Option<SystemTime>,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    length: u64,
    modified: Option<SystemTime>,
}

#[cfg(not(any(windows, unix)))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    length: u64,
    modified: Option<SystemTime>,
}

#[cfg(windows)]
fn file_identity(metadata: &std::fs::Metadata) -> Result<FileIdentity, String> {
    use std::os::windows::fs::MetadataExt;
    Ok(FileIdentity {
        created: metadata.creation_time(),
        written: metadata.last_write_time(),
        length: metadata.len(),
        attributes: metadata.file_attributes(),
        modified: metadata.modified().ok(),
    })
}

#[cfg(unix)]
fn file_identity(metadata: &std::fs::Metadata) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

#[cfg(not(any(windows, unix)))]
fn file_identity(metadata: &std::fs::Metadata) -> Result<FileIdentity, String> {
    Ok(FileIdentity {
        length: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

fn protected_user_folders() -> Vec<PathBuf> {
    let mut candidates = vec![
        dirs::desktop_dir(),
        dirs::document_dir(),
        dirs::download_dir(),
        dirs::picture_dir(),
        dirs::video_dir(),
        dirs::audio_dir(),
        dirs::config_dir(),
        dirs::data_dir(),
        dirs::data_local_dir(),
    ];

    for key in ["APPDATA", "LOCALAPPDATA", "OneDrive"] {
        if let Some(path) = std::env::var_os(key).map(PathBuf::from) {
            candidates.push(Some(path));
        }
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(Some(home.join("OneDrive")));
    }

    candidates
        .into_iter()
        .flatten()
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect()
}

// Reject paths outside the user's home directory, the home itself, links/reparse
// points, and entire standard folders that contain important user or app data.
// Individual files or subfolders can still be moved after the confirmation dialog.
pub fn validate_eat_path(path: &Path) -> Result<PathBuf, String> {
    let link_metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {:?}: {}", path, error))?;
    if is_reparse_point(&link_metadata) {
        return Err(format!("links and junctions cannot be eaten: {:?}", path));
    }

    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("could not resolve {:?}: {}", path, error))?;
    let home = dirs::home_dir().ok_or_else(|| "no home directory is available".to_string())?;
    let home_canonical = std::fs::canonicalize(&home).unwrap_or(home);
    if canonical == home_canonical || !canonical.starts_with(&home_canonical) {
        return Err(format!(
            "path is outside the allowed home folder: {:?}",
            canonical
        ));
    }

    if protected_user_folders()
        .iter()
        .any(|root| root == &canonical)
    {
        return Err(format!(
            "this is a protected standard folder; drag a specific file or subfolder instead: {:?}",
            canonical
        ));
    }

    Ok(canonical)
}

fn current_target_identity(path: &Path) -> Result<FileIdentity, String> {
    let canonical = validate_eat_path(path)?;
    if canonical != path {
        return Err("the target location changed".into());
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {:?}: {}", path, error))?;
    if is_reparse_point(&metadata) || !(metadata.is_file() || metadata.is_dir()) {
        return Err("the target is no longer a regular file or folder".into());
    }
    let identity = file_identity(&metadata)?;

    // Check again around the metadata read so a swapped link or path is rejected.
    if validate_eat_path(path)? != path {
        return Err("the target location changed".into());
    }
    let after = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {:?}: {}", path, error))?;
    if is_reparse_point(&after) || file_identity(&after)? != identity {
        return Err("the target item changed".into());
    }
    Ok(identity)
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut amount = bytes as f64;
    let mut unit = 0usize;
    while amount >= 1024.0 && unit < UNITS.len() - 1 {
        amount /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{amount:.1} {}", UNITS[unit])
    }
}

fn direct_folder_count(path: &Path) -> String {
    let Ok(entries) = std::fs::read_dir(path) else {
        return "Contents could not be counted".into();
    };
    let mut count = 0usize;
    for entry in entries {
        if entry.is_err() {
            return "Contents could not be counted".into();
        }
        count += 1;
        if count >= MAX_DIRECT_FOLDER_ITEMS {
            return format!("At least {MAX_DIRECT_FOLDER_ITEMS} items at the top level");
        }
    }
    format!("{count} items at the top level")
}

fn describe_and_identify_target(path: &Path) -> Result<(String, FileIdentity), String> {
    let identity = current_target_identity(path)?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {:?}: {}", path, error))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let full_path = path.to_string_lossy();

    let details = if metadata.is_dir() {
        format!(
            "• Folder: {name}\n  Location: {full_path}\n  Contents: {} — the whole folder and everything inside it will move",
            direct_folder_count(path)
        )
    } else if metadata.is_file() {
        format!(
            "• File: {name}\n  Location: {full_path}\n  Size: {}",
            format_size(metadata.len())
        )
    } else {
        return Err(format!(
            "only regular files and folders can be moved: {:?}",
            path
        ));
    };

    if current_target_identity(path)? != identity {
        return Err("a target changed while its details were being prepared".into());
    }
    Ok((details, identity))
}

async fn ask_before_eating(app: &AppHandle, details: String) -> Result<bool, String> {
    let message = format!(
        "Clawd is asking to move these items to the Recycle Bin. Review each name, location, and size.\n\n{details}\n\nFolders move with all their contents. This cannot be undone from Clawd after the Recycle Bin is emptied. Select Yes to move them, or No to keep them."
    );
    let (sender, receiver) = oneshot::channel();
    app.dialog()
        .message(message)
        .title("Review before Clawd moves files")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::YesNo)
        .show(move |answer| {
            let _ = sender.send(answer);
        });
    receiver.await.map_err(|_| {
        "the file confirmation dialog closed unexpectedly; nothing was moved".to_string()
    })
}

pub async fn eat_paths(
    app: AppHandle,
    state: SharedState,
    paths: Vec<PathBuf>,
) -> Result<usize, String> {
    if paths.is_empty() || paths.len() > MAX_PATHS {
        set_state(&app, &state, PetState::IdleLiving);
        return Err(format!("eat accepts between 1 and {MAX_PATHS} paths"));
    }

    let targets = match paths
        .iter()
        .map(|path| validate_eat_path(path))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(targets) => targets,
        Err(error) => {
            set_state(&app, &state, PetState::IdleLiving);
            return Err(error);
        }
    };
    let inspected = match targets
        .iter()
        .map(|path| describe_and_identify_target(path))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(items) => items,
        Err(error) => {
            set_state(&app, &state, PetState::IdleLiving);
            return Err(error);
        }
    };
    let details = inspected
        .iter()
        .map(|(description, _)| description.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let identities: Vec<FileIdentity> = inspected
        .into_iter()
        .map(|(_, identity)| identity)
        .collect();

    match ask_before_eating(&app, details).await {
        Ok(true) => {}
        Ok(false) => {
            set_state(&app, &state, PetState::IdleLiving);
            return Err("Cancelled; no files were moved.".into());
        }
        Err(error) => {
            set_state(&app, &state, PetState::IdleLiving);
            return Err(error);
        }
    }

    // Revalidate every target's canonical path and file identity after confirmation.
    for (target, identity) in targets.iter().zip(&identities) {
        match current_target_identity(target) {
            Ok(current) if current == *identity => {}
            Ok(_) => {
                set_state(&app, &state, PetState::IdleLiving);
                return Err("a target changed after confirmation; nothing was moved".into());
            }
            Err(error) => {
                set_state(&app, &state, PetState::IdleLiving);
                return Err(format!(
                    "a target changed after confirmation; nothing was moved: {error}"
                ));
            }
        }
    }

    set_state(&app, &state, PetState::Eating);

    let mut ok = 0usize;
    let mut had_error = false;
    for (path, identity) in targets.iter().zip(&identities) {
        // Keep the final check adjacent to the path-based Recycle Bin operation.
        if let Err(error) = current_target_identity(path).and_then(|current| {
            if current == *identity {
                Ok(())
            } else {
                Err("the selected item changed after confirmation".into())
            }
        }) {
            had_error = true;
            log::warn!("skipped changed eat target {:?}: {}", path, error);
            continue;
        }
        match trash::delete(path) {
            Ok(()) => ok += 1,
            Err(error) => {
                had_error = true;
                log::warn!("trash::delete failed for {:?}: {}", path, error);
            }
        }
    }

    sleep(Duration::from_millis(1200)).await;

    if had_error && ok == 0 {
        set_state(&app, &state, PetState::WorkingConfused);
        sleep(Duration::from_millis(1500)).await;
    } else {
        set_state(&app, &state, PetState::Happy);
        sleep(Duration::from_millis(1500)).await;
    }

    set_state(&app, &state, PetState::IdleLiving);
    Ok(ok)
}

fn set_state(app: &AppHandle, state: &SharedState, next: PetState) {
    *state.write() = next;
    let payload: crate::state::StatePayload = next.into();
    if let Err(error) = app.emit("state-changed", payload) {
        log::warn!("emit state-changed failed: {}", error);
    }
}

#[tauri::command]
pub async fn eat_files(
    paths: Vec<String>,
    app: AppHandle,
    state: State<'_, SharedState>,
    window: WebviewWindow,
) -> Result<usize, String> {
    if window.label() != "main" {
        return Err("file eating is available only from the pet window".into());
    }
    if paths.is_empty() || paths.len() > MAX_PATHS || paths.iter().any(|path| path.len() > 4096) {
        return Err(format!("eat accepts between 1 and {MAX_PATHS} short paths"));
    }
    let buffers: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let shared = state.inner().clone();
    eat_paths(app, shared, buffers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_directory_itself_is_not_an_eatable_path() {
        if let Some(home) = dirs::home_dir() {
            assert!(validate_eat_path(&home).is_err());
        }
    }

    #[test]
    fn standard_user_folders_are_protected() {
        for folder in protected_user_folders() {
            assert!(
                validate_eat_path(&folder).is_err(),
                "standard folder should be protected: {:?}",
                folder
            );
        }
    }
}
