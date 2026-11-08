use crate::state::{ExternalUntil, PetState, SharedState, StatePayload};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Default)]
pub struct ActivityTracker(Arc<Mutex<ActivityFlags>>);

#[derive(Default)]
struct ActivityFlags {
    assistant_busy: bool,
    typing_active: bool,
    applied_state: Option<PetState>,
}

impl ActivityTracker {
    pub fn set_assistant_busy(&self, active: bool) {
        self.0.lock().assistant_busy = active;
    }

    fn set_typing_active(&self, active: bool) {
        self.0.lock().typing_active = active;
    }

    fn snapshot(&self) -> (Option<PetState>, Option<PetState>) {
        let flags = self.0.lock();
        let desired = if flags.assistant_busy {
            Some(PetState::WorkingThinking)
        } else if flags.typing_active {
            Some(PetState::WorkingTyping)
        } else {
            None
        };
        (desired, flags.applied_state)
    }

    fn set_applied_state(&self, state: Option<PetState>) {
        self.0.lock().applied_state = state;
    }
}

pub fn spawn_activity_coordinator(
    app: AppHandle,
    state: SharedState,
    external_until: ExternalUntil,
    tracker: ActivityTracker,
) {
    keyboard::start();

    tauri::async_runtime::spawn(async move {
        loop {
            tracker.set_typing_active(keyboard::recently_typing());
            let (desired, applied) = tracker.snapshot();
            let current = *state.read();

            match desired {
                Some(next) if !is_protected_state(current) => {
                    if current != next {
                        publish_state(&app, &state, &external_until, next);
                    }
                    tracker.set_applied_state(Some(next));
                }
                Some(_) => {
                    // Let transient pet actions finish before changing their pose.
                    tracker.set_applied_state(None);
                }
                None => {
                    if applied == Some(current) {
                        publish_state(&app, &state, &external_until, PetState::IdleLiving);
                    }
                    tracker.set_applied_state(None);
                }
            }

            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    });
}

fn is_protected_state(state: PetState) -> bool {
    matches!(
        state,
        PetState::Eating
            | PetState::Happy
            | PetState::Dizzy
            | PetState::Dragging
            | PetState::GoingAway
            | PetState::Disconnected
            | PetState::Notification
    )
}

fn publish_state(
    app: &AppHandle,
    state: &SharedState,
    external_until: &ExternalUntil,
    next: PetState,
) {
    *state.write() = next;
    *external_until.lock() = Instant::now() + Duration::from_secs(3);
    let payload: StatePayload = next.into();
    if let Err(error) = app.emit("state-changed", payload) {
        log::warn!("emit activity state failed: {}", error);
    }
}

#[cfg(windows)]
mod keyboard {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::SystemInformation::GetTickCount64;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, HHOOK, HOOKPROC, MSG, PM_NOREMOVE, PM_REMOVE, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    const TYPING_GRACE_MS: u64 = 1_400;
    static LAST_KEYDOWN_TICK: AtomicU64 = AtomicU64::new(0);

    pub fn recently_typing() -> bool {
        let last = LAST_KEYDOWN_TICK.load(Ordering::Relaxed);
        last != 0 && unsafe { GetTickCount64() }.saturating_sub(last) <= TYPING_GRACE_MS
    }

    pub fn start() {
        if let Err(error) = thread::Builder::new()
            .name("clawd-key-activity".into())
            .spawn(keyboard_hook_thread)
        {
            log::warn!("could not start local keyboard activity monitor: {}", error);
        }
    }

    unsafe extern "system" fn keyboard_hook_proc(
        code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        // Do not inspect l_param (which contains the key code), translate keys, or retain text.
        if code >= 0 && matches!(w_param.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
            LAST_KEYDOWN_TICK.store(GetTickCount64(), Ordering::Relaxed);
        }
        CallNextHookEx(HHOOK::default(), code, w_param, l_param)
    }

    fn keyboard_hook_thread() {
        unsafe {
            let mut message = MSG::default();
            // Create a message queue before installing the low-level hook.
            let _ = PeekMessageW(&mut message, HWND::default(), 0, 0, PM_NOREMOVE);

            let instance = match GetModuleHandleW(PCWSTR::null()) {
                Ok(module) => HINSTANCE(module.0),
                Err(error) => {
                    log::warn!(
                        "could not get app module handle for keyboard activity: {}",
                        error
                    );
                    return;
                }
            };
            let hook_proc: HOOKPROC = Some(keyboard_hook_proc);
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, hook_proc, instance, 0) {
                Ok(hook) => hook,
                Err(error) => {
                    log::warn!("could not install local keyboard activity hook: {}", error);
                    return;
                }
            };

            loop {
                while PeekMessageW(&mut message, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
                    if message.message == WM_QUIT {
                        let _ = UnhookWindowsHookEx(hook);
                        return;
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                thread::sleep(Duration::from_millis(40));
            }
        }
    }
}

#[cfg(not(windows))]
mod keyboard {
    pub fn start() {}

    pub fn recently_typing() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_work_takes_priority_over_typing_and_releases_back_to_typing() {
        let tracker = ActivityTracker::default();
        tracker.set_typing_active(true);
        assert_eq!(tracker.snapshot().0, Some(PetState::WorkingTyping));

        tracker.set_assistant_busy(true);
        assert_eq!(tracker.snapshot().0, Some(PetState::WorkingThinking));

        tracker.set_assistant_busy(false);
        assert_eq!(tracker.snapshot().0, Some(PetState::WorkingTyping));
    }

    #[test]
    fn no_activity_leaves_random_working_poses_unclaimed() {
        let tracker = ActivityTracker::default();
        assert_eq!(tracker.snapshot(), (None, None));
    }
}
