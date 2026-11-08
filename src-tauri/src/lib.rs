// Clawd desktop pet
mod assistant;
mod file_eater;
mod http_api;
mod mood;
mod pet_controller;
mod pet_pack;
mod state;
mod tray;
mod window_tracker;

use crate::mood::{new_shared_mood, SharedMood};
use crate::pet_pack::{new_shared_pack, SharedPack};
use crate::state::{
    new_external_until, new_shared_state, ExternalUntil, PetState, SharedState, StatePayload,
};
use crate::window_tracker::{new_shared_windows, SharedWindows};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::PhysicalPosition;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Default)]
struct ChatHoverState(Arc<Mutex<ChatHoverFlags>>);

#[derive(Default)]
struct ChatHoverFlags {
    pet: bool,
    panel: bool,
}

#[tauri::command]
fn set_assistant_hover(
    source: String,
    inside: bool,
    app: AppHandle,
    hover: tauri::State<'_, ChatHoverState>,
) -> Result<(), String> {
    {
        let mut flags = hover.0.lock();
        match source.as_str() {
            "main" => flags.pet = inside,
            "assistant" => flags.panel = inside,
            _ => return Err("unknown hover source".into()),
        }
    }
    if inside {
        show_assistant_at_pet(&app)?;
    } else {
        let flags = hover.0.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(750)).await;
            let should_hide = {
                let current = flags.lock();
                !current.pet && !current.panel
            };
            if should_hide {
                if let Some(window) = app.get_webview_window("assistant") {
                    let _ = window.hide();
                }
            }
        });
    }
    Ok(())
}

fn show_assistant_at_pet(app: &AppHandle) -> Result<(), String> {
    let pet = app
        .get_webview_window("main")
        .ok_or_else(|| "pet window is unavailable".to_string())?;
    let panel = app
        .get_webview_window("assistant")
        .ok_or_else(|| "assistant window is unavailable".to_string())?;
    let pet_pos = pet.outer_position().map_err(|e| e.to_string())?;
    let pet_size = pet.outer_size().map_err(|e| e.to_string())?;
    let panel_size = panel.outer_size().map_err(|e| e.to_string())?;
    let mut x = pet_pos.x + (pet_size.width as i32 * 3 / 4) - panel_size.width as i32;
    let mut y = pet_pos.y + (pet_size.height as i32 / 2) - panel_size.height as i32 + 45;
    if let Ok(Some(monitor)) = pet.current_monitor() {
        let origin = monitor.position();
        let size = monitor.size();
        let max_x = (origin.x + size.width as i32 - panel_size.width as i32).max(origin.x);
        let max_y = (origin.y + size.height as i32 - panel_size.height as i32).max(origin.y);
        x = x.clamp(origin.x, max_x);
        y = y.clamp(origin.y, max_y);
    }
    panel
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    panel.show().map_err(|e| e.to_string())
}

#[tauri::command]
fn set_state_cmd(
    state_key: String,
    app: AppHandle,
    state: tauri::State<'_, SharedState>,
    external_until: tauri::State<'_, ExternalUntil>,
) -> Result<(), String> {
    let next =
        PetState::from_key(&state_key).ok_or_else(|| format!("unknown state: {}", state_key))?;
    *external_until.lock() = Instant::now() + Duration::from_secs(3);
    *state.write() = next;
    let payload: StatePayload = next.into();
    app.emit("state-changed", payload)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_state_cmd(state: tauri::State<'_, SharedState>) -> StatePayload {
    let s = *state.read();
    s.into()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shared = new_shared_state();
    let windows = new_shared_windows();
    let external_until = new_external_until();
    let pack = new_shared_pack();
    let mood = new_shared_mood();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(shared)
        .manage(windows)
        .manage(external_until)
        .manage(pack)
        .manage(mood)
        .manage(ChatHoverState::default())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            tray::setup_tray(&app.handle())?;

            let assistant = assistant::AssistantState::new(app.path().app_data_dir()?);
            app.manage(assistant.clone());
            assistant::spawn_reminder_scheduler(app.handle().clone(), assistant);

            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_skip_taskbar(true);
                #[cfg(debug_assertions)]
                {
                    w.open_devtools();
                }
            }

            let shared = app.state::<SharedState>().inner().clone();
            let windows = app.state::<SharedWindows>().inner().clone();
            let external_until = app.state::<ExternalUntil>().inner().clone();
            let pack = app.state::<SharedPack>().inner().clone();
            let mood = app.state::<SharedMood>().inner().clone();

            #[cfg(windows)]
            let self_hwnd: isize = app
                .get_webview_window("main")
                .and_then(|w| w.hwnd().ok().map(|h| h.0 as isize))
                .unwrap_or(0);
            #[cfg(not(windows))]
            let self_hwnd: isize = 0;

            window_tracker::spawn_window_poller(windows.clone(), self_hwnd);
            mood::spawn_mood_thread(mood.clone());
            pet_controller::spawn_auto_cycle(
                app.handle().clone(),
                shared.clone(),
                external_until.clone(),
                pack.clone(),
                mood,
            );
            pet_controller::spawn_walker(app.handle().clone(), shared.clone(), windows, pack);
            http_api::spawn_http_server(app.handle().clone(), shared, external_until);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_state_cmd,
            get_state_cmd,
            set_assistant_hover,
            assistant::assistant_settings,
            assistant::save_ollama_api_key,
            assistant::clear_ollama_api_key,
            assistant::set_ollama_model,
            assistant::set_enabled_folders,
            assistant::list_todos,
            assistant::add_todo,
            assistant::complete_todo,
            assistant::delete_todo,
            assistant::search_files,
            assistant::reveal_file,
            assistant::assistant_chat,
            file_eater::eat_files,
            pet_pack::list_packs_cmd,
            pet_pack::get_pack_cmd,
            pet_pack::set_pack_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
