# Clawd

**A small Windows desktop pet with a private, task-focused AI companion.** Clawd wanders across your desktop, reacts to your activity, and can chat through a compact speech panel.

<p align="center">
  <img src="src/assets/clawd-idle-living.svg" alt="Clawd at rest" width="92" />
  <img src="src/assets/clawd-working-typing.svg" alt="Clawd working at a laptop" width="92" />
  <img src="src/assets/clawd-crab-walking.svg" alt="Clawd walking" width="92" />
</p>

**Publisher metadata:** NM Developers · **Platform:** Windows 10/11 (64-bit) · **Runtime:** Tauri 2 + Rust

## Downloads

The latest public release is [Clawd v0.2.0](https://github.com/nadeemmhdm/clawd-window/releases/tag/v0.2.0). Download either installer from its release page:

- **MSI:** `clawd_0.2.0_x64_en-US.msi`
- **Setup EXE:** `clawd_0.2.0_x64-setup.exe`

An updated **v0.2.2 candidate** is being prepared on the `feature/ollama-pet-assistant` branch. Once the Windows build completes, its temporary installer artifact will be available from the branch’s [Windows Actions workflow](https://github.com/nadeemmhdm/clawd-window/actions/workflows/windows-installer.yml). It is not a public release.

The installer metadata identifies **NM Developers** as the publisher. This metadata is not a digital signature; until a code-signing certificate is configured, Windows may still show an unsigned-app or SmartScreen warning.

## What Clawd does

- **Lives on your desktop:** Clawd takes more frequent strolls, climbs window edges, follows your cursor with its eyes, and reacts when clicked or dragged.
- **Chats like a pet:** Click Clawd to open the dialogue panel; simply moving the cursor over it will not open chat. The message box stays ready after every reply, and only the latest exchange is shown while conversation context remains in memory for the current session.
- **Connects to Ollama Cloud:** Chat and requested assistant actions run through Ollama’s chat API. The API key is stored in Windows Credential Manager, not in browser storage or project files.
- **Shows activity poses:** Clawd uses its animated laptop-typing pose while Windows keyboard activity is detected or an AI reply is being generated. Keyboard activity is measured locally; key values and typed text are not captured, stored, or sent.
- **Manages tasks and reminders:** Tell Clawd what you need to do or remember in chat; it can add a task without requiring the words “task” or “TODO.” Ask explicitly for a reminder to schedule one. Tasks and reminders are stored locally and checked against your computer’s local time while Clawd is running. The Tasks view is optional for reviewing and completing items.
- **Finds filenames:** Optionally allow access to Downloads, Documents, Pictures, Videos, or Music. Search matches meaningful words across spaces, hyphens, underscores, and combined words; “PDF” acts as a file-type filter. It returns names and locations only, never reads document contents, and stays on the computer. Choose **Open location** to reveal a match in File Explorer.
- **Protects file deletion:** Dropping a file on Clawd opens a native confirmation with the target’s name, location, and size. Confirmed items move to the Recycle Bin. Important standard folders and links are protected.
- **Starts when you sign in:** An on/off setting in Clawd’s Settings controls Windows startup.

## First-time setup

1. Install and launch Clawd. Its crab appears on the desktop and in the system tray.
2. Click the pet to open the assistant, then open **Settings**.
3. Paste an Ollama Cloud API key and select a model available to your account. Clawd verifies that the key was saved to Windows Credential Manager; the **Key saved** label means it is stored, not that a live request has already succeeded.
4. Return to chat and ask a question. The message box remains available for follow-up questions after each reply.

### If chat cannot connect

- If Settings says **Not connected**, save the API key again. If it reports that the credential store is unavailable, restart Clawd and try again.
- If Settings says **Saved key invalid**, replace the value with a valid Ollama Cloud API key.
- If Ollama rejects the key, replace it in Settings. A saved key is not the same as a validated Ollama session.
- If a request times out, check your internet connection. For model or request errors, Clawd now displays a short provider error when Ollama supplies one.

## Controls

- **Hover over Clawd:** Clawd follows the cursor with its eyes; chat stays closed.
- **Click Clawd:** Play its happy animation and open the assistant.
- **Drag Clawd:** Move it around the screen.
- **Right-click the tray icon:** Show or hide the pet, sleep or wake it, reset its position, or exit.
- **Drop a file on Clawd:** Review the confirmation dialog before moving it to the Recycle Bin.

## Privacy and safety

- Chat messages are sent to Ollama Cloud to generate replies. Conversation history is kept in memory only and is not saved as a chat transcript.
- The API key is kept in the operating system credential store. Clawd does not store it in the WebView, local settings file, or repository.
- Keyboard activity is used only as a local animation signal. Clawd does not inspect key values or record typed text.
- Filename search is disabled until folders are explicitly enabled. Flexible word matching stays on the computer; document contents are not read, and results are not uploaded.
- Tasks and reminders remain in Clawd’s local app data. Reminders are shown while the app is running.
- File-eater actions require confirmation and use the Recycle Bin. API-driven deletion is disabled by default; protected folders cannot be targeted.
- The local control API is bound to loopback, requires a per-install bearer token, and has a restrictive request policy.

## Build from source

### Requirements

- Windows 10 or 11, 64-bit
- Rust 1.90 or newer
- Microsoft C++ Build Tools
- WebView2 Runtime (included with current Windows 11 installations)

### Build

```powershell
git clone --branch feature/ollama-pet-assistant https://github.com/nadeemmhdm/clawd-window.git
cd clawd-window
cargo install tauri-cli --version "^2.0" --locked
cargo tauri dev
```

Create release installers with:

```powershell
cargo tauri build
```

The MSI and NSIS installers are written under `src-tauri/target/release/bundle/`. Pushing to the `feature/ollama-pet-assistant` branch runs the Windows installer workflow and uploads an Actions artifact. That workflow does not create a GitHub Release.
Before uploading, the Windows workflow asks Windows Installer to expand the MSI and verifies that it contains an install payload.

## Project layout

- `src/` — assistant and pet HTML, CSS, JavaScript, and SVG animations
- `src-tauri/src/assistant.rs` — Ollama connection, tasks, reminders, and filename search
- `src-tauri/src/activity.rs` — local typing and AI-work animation signals
- `src-tauri/src/file_eater.rs` — file review, protections, and Recycle Bin handling
- `src-tauri/src/http_api.rs` — authenticated loopback API
- `src-tauri/tauri.conf.json` — app windows and bundle metadata

## License

No license file is currently included in this repository.
