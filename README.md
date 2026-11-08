<div align="center">

<img src="src-tauri/icons/icon.png" alt="Clawd" width="180"/>

# Clawd

**A cute pixel-art desktop pet for Windows.**
Inspired by Shimeji — a small crab named Clawd lives on your desktop, walks across your screen, climbs your application windows, and eats files you drag onto him (sending them safely to the Recycle Bin).

[![Tauri](https://img.shields.io/badge/Tauri-2.12-FFC131?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-2021-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Windows](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows&logoColor=white)](#)
[![Release](https://img.shields.io/badge/release-0.1.0-blue)](#install)

</div>

---

## Showcase

<table align="center">
  <tr>
    <td align="center">
      <img src="src/assets/clawd-idle-living.svg" width="120" alt="idle"/><br/>
      <sub><b>Idle</b></sub>
    </td>
    <td align="center">
      <img src="src/assets/clawd-crab-walking.svg" width="120" alt="walking"/><br/>
      <sub><b>Walking</b></sub>
    </td>
    <td align="center">
      <img src="src/assets/clawd-happy.svg" width="120" alt="happy"/><br/>
      <sub><b>Happy</b></sub>
    </td>
    <td align="center">
      <img src="src/assets/clawd-working-typing.svg" width="120" alt="typing"/><br/>
      <sub><b>Working</b></sub>
    </td>
    <td align="center">
      <img src="src/assets/clawd-sleeping.svg" width="120" alt="sleeping"/><br/>
      <sub><b>Sleeping</b></sub>
    </td>
  </tr>
</table>

23 hand-crafted pixel-art animations — open the SVG files in `src/assets/` to see them animate.

---

## Features

- **Always-on-top transparent pet** — sits on your desktop without a window frame.
- **Mouse interaction** — click to make Clawd happy, drag to move him around, eyes follow your cursor when nearby.
- **Window climbing** *(Shimeji-style)* — Clawd walks up the sides of any open application window, walks along the top edge, then turns the corner and climbs back down.
- **Screen edge climbing** — climbs the left/right monitor edges and walks back across the top.
- **Review-first file eater** — before anything moves, a native dialog lists each file's name, full location, and size; for folders it warns that all contents move too. The home folder, standard Windows user folders, links, and junctions are protected.
- **Hover chat** — move the cursor over Clawd to open a small assistant chat window; the pet's movement window stays separate.
- **Ollama Cloud chat** — connect an API key from Settings; requests are sent from the Rust backend to Ollama's OpenAI-compatible API, never directly from browser code.
- **Tasks and reminders** — ask Clawd to add a TODO or set a reminder in local system time; tasks are saved locally and reminders fire while Clawd is running.
- **Filename search** — optionally allow-list Downloads, Documents, Pictures, Videos, and Music; Clawd searches names only and can reveal a match in the file manager without opening or reading it.
- **Privacy controls** — chat history is session-only, tasks remain local, the API key uses the operating system credential store, and file search is off until folders are enabled.
- **Auto-cycle idle states** — Clawd randomly switches between walking, working, sleeping, etc. when idle.
- **Multi-monitor + HiDPI aware** — pet positions correctly across monitors with different DPI scaling (100%, 150%, 200%).
- **Tray menu** — Show / Hide / Force Sleep / Wake Up / Reset Position / Exit.
- **Start with Windows** — an on/off toggle in Settings makes the pet appear automatically when you sign in.
- **Single instance** — running Clawd a second time just focuses the existing one.

---

## Install

### Option 1 — Pre-built installer (easiest)

Download the **Clawd-Windows-Installers** artifact from the **[Actions tab](https://github.com/nadeemmhdm/clawd-window/actions/workflows/windows-installer.yml)** for the `feature/ollama-pet-assistant` branch, then run one of:

| File | Type | Size | Notes |
|---|---|---|---|
| `clawd_0.1.0_x64-setup.exe` | NSIS installer | ~1.9 MB | Recommended |
| `clawd_0.1.0_x64_en-US.msi` | MSI installer | ~2.9 MB | For enterprise / silent install |

> Windows SmartScreen may warn the binary isn't signed — click **More info → Run anyway**.

### Option 2 — Build from source

See [Build from source](#build-from-source) below.

---

## Usage

After install, **Clawd** appears as a floating crab near the bottom-right of your screen and as a tray icon in the system tray.

| Action | Result |
|---|---|
| **Click** Clawd | Plays happy animation |
| **Drag** Clawd | Move him anywhere on screen |
| Hover Clawd | Opens the chat panel with a message box and Send button |
| Ask for a TODO/reminder | Adds a local task; reminders use your computer's local time while Clawd is running |
| Ask to find a filename | Searches only the folders enabled in Settings; choose **Open location** to select it in the file manager |
| **Drop file** on Clawd | Review the name, full path, and size in a confirmation dialog; choose Yes to move it to the Recycle Bin or No to keep it |
| Drop an entire standard user folder | Rejected; choose a specific file or subfolder instead |
| **Right-click** tray icon | Open menu (Show/Hide/Sleep/Reset/Exit) |
| Move cursor near Clawd | Eyes track your cursor |
| Leave Clawd alone | Auto-cycles through animations; sleeps after ~5 min idle |
| Walk into a window edge | Clawd climbs it, walks across the top, climbs down the other side |

---

## Assistant setup and privacy

Hover over Clawd and open **Settings**. Create an API key in [Ollama Cloud](https://docs.ollama.com/cloud), paste it into the key field, and choose a model supported by Ollama's [chat completions API](https://docs.ollama.com/api/openai-compatibility). The key is stored in the operating system credential store and is never saved in the WebView or repository.

The assistant sends your chat message to Ollama Cloud. Chat history is kept in memory only. Tasks and reminders are stored in the app's local data folder. Reminders are checked against the computer's system clock and appear in Clawd while the app is running.

In **Settings → Start with Windows**, turn on **Show the pet when I sign in** to have Clawd appear automatically after you sign in. The setting is off until you enable it and can be turned off there at any time.

Filename search is disabled until you select one or more known folders in Settings. Search runs locally, follows no symlinks, and returns filenames and locations only—not file contents. A file is never opened automatically: choose **Open location** to reveal it in the file manager. The assistant cannot run shell commands or modify files.

The local control API remains bound to `127.0.0.1`, requires a generated bearer token, and limits request size and file-eater batches. API-driven file deletion is disabled by default; if explicitly enabled, it still shows the same confirmation dialog. The file eater rejects the home folder, standard user-data/app-data roots, symbolic links, and Windows junctions. After you confirm, it revalidates every target before moving anything. The Tauri WebView uses a restrictive Content Security Policy.

---

## Build from source

### Prerequisites

- **Windows 10/11** (64-bit)
- **[Rust](https://rustup.rs)** ≥ 1.90 (`rustup default stable`)
- **[Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)** — required for linking on Windows
- **[WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)** — pre-installed on Windows 11

Verify:
```powershell
rustc --version    # 1.90 or newer
cargo --version
```

### Clone & install Tauri CLI

```powershell
git clone https://github.com/Hert4/clawd-window.git
cd clawd-window
cargo install tauri-cli --version "^2.0" --locked
```

### Run in dev mode

```powershell
cargo tauri dev
```

First build takes ~5 minutes (compiling Tauri + dependencies). Subsequent runs are fast — Tauri auto-reloads on file changes.

### Build a release installer

```powershell
cargo tauri build
```

The build produces two installers in `src-tauri/target/release/bundle/`:

```
src-tauri/target/release/bundle/
├── msi/clawd_0.1.0_x64_en-US.msi      # MSI for enterprise / silent install
└── nsis/clawd_0.1.0_x64-setup.exe     # NSIS installer (recommended)
```

Both come with a multi-resolution icon (16/24/32/48/64/128/256) embedded in the `.exe`, so the Start Menu / Desktop shortcut shows Clawd at every DPI. Pushing to the `feature/ollama-pet-assistant` branch triggers the **Build Windows Installers** GitHub Actions workflow, which builds both formats on a Windows runner and uploads a downloadable artifact.

### Regenerate icons (optional)

If you change `src-tauri/icons/icon-source.png`, regenerate all sizes + the `.ico`:

```powershell
cargo tauri icon src-tauri/icons/icon-source.png
```

---

## Project structure

```
clawd-window/
├── src/                              # Frontend (vanilla HTML/CSS/JS)
│   ├── index.html                    # <object> tag swaps SVG per state
│   ├── pet.js                        # State listener, drag/click, eye-tracking
│   ├── style.css                     # Transparent body, pixelated rendering
│   ├── assistant.html/js/css          # Separate chat, tasks, and privacy settings window
│   └── assets/                       # 23 hand-crafted SVG animations
│
├── src-tauri/                        # Backend (Rust)
│   ├── src/
│   │   ├── lib.rs                    # App entry, plugin setup, command registration
│   │   ├── assistant.rs              # Ollama Cloud, local tasks/reminders, allowlisted filename search
│   │   ├── main.rs                   # Binary entry
│   │   ├── state.rs                  # PetState enum + Arc<RwLock>
│   │   ├── pet_controller.rs         # Walking, climbing, gravity tick (60 Hz)
│   │   ├── window_tracker.rs         # Win32 EnumWindows polling
│   │   ├── file_eater.rs             # review dialog, protected folders, then move to Recycle Bin
│   │   ├── http_api.rs               # Loopback API protected by a generated bearer token
│   │   └── tray.rs                   # Tray icon + menu
│   ├── icons/                        # App icons (PNG + ICO + ICNS)
│   ├── Cargo.toml                    # Rust dependencies
│   └── tauri.conf.json               # Window/bundle config
│
└── assest/                           # Original SVG sources (kept for reference)
```

---

## Tech stack

| Layer | Tech |
|---|---|
| Window / runtime | [Tauri 2.12](https://tauri.app) — Rust backend + WebView2 frontend |
| Backend language | [Rust 2021](https://www.rust-lang.org) |
| Frontend | Vanilla HTML/CSS/JS (no bundler) |
| Pet rendering | SVG with `@keyframes` CSS animations, swapped via `<object data>` to avoid CSS scope collisions |
| Win32 integration | [`windows-rs`](https://crates.io/crates/windows) — `EnumWindows`, `MonitorFromPoint`, `GetMonitorInfoW`, `DwmGetWindowAttribute` |
| File deletion | [`trash`](https://crates.io/crates/trash) — moves to Recycle Bin (recoverable) |
| Async runtime | [`tokio`](https://tokio.rs) |
| State sync | [`parking_lot::RwLock`](https://crates.io/crates/parking_lot) wrapped in `Arc` |

---

## How the climbing logic works

Every 16 ms, the walker tick:

1. Reads pet's current position and the monitor's work area (`MonitorFromPoint` + `GetMonitorInfoW`, properly handling multi-DPI).
2. Detects which `PetState` it's in (Walking / Climbing / Falling / Idle / …).
3. **Walking**: moves pet horizontally; if pet's right edge touches the left edge of an open window, transitions to `Climbing { hwnd, edge: Left }` and applies gravity = false.
4. **Climbing**: moves pet vertically along the window's edge. When pet reaches the top, runs a 14-tick lateral interpolation (`corner_turn`) so the pet smoothly walks onto the top of the window, then transitions back to `Walking` along the top.
5. **Window destroyed** while climbing: pet falls back to the floor.
6. **Gravity**: when no edge/wall is grabbed, pet falls toward the nearest floor — either screen taskbar top, or the top of any window the pet is centered over.

All measurements use **physical pixels** from `outer_size()` so positioning stays correct when the pet crosses monitors with different DPI scaling.

---

## License

This project is provided as-is. The Clawd character pixel-art SVGs in `src/assets/` are the original work of the project author.

---

<div align="center">
<sub>Made with Tauri · Rust · WebView2</sub>
</div>
