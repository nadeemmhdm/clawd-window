"use strict";

const tauri = window.__TAURI__ || {};
const invoke = tauri.core && tauri.core.invoke ? tauri.core.invoke : async () => { throw new Error("Clawd desktop bridge is unavailable."); };
const listen = tauri.event && tauri.event.listen ? tauri.event.listen : async () => () => {};
const conversation = [];
const views = ["chat-view", "tasks-view", "settings-view"];
const messagesEl = document.getElementById("messages");
const promptEl = document.getElementById("prompt");
const sendEl = document.getElementById("send");
const composerEl = document.getElementById("chat-form");
const askAgainEl = document.getElementById("ask-again");
let toastTimer = null;

function showView(id) {
  for (const viewId of views) {
    const el = document.getElementById(viewId);
    const active = viewId === id;
    el.hidden = !active;
    el.classList.toggle("active", active);
  }
  if (id === "tasks-view") refreshTodos();
  if (id === "settings-view") refreshSettings();
  if (id === "chat-view") setTimeout(() => (composerEl.hidden ? askAgainEl : promptEl).focus(), 20);
}

function addMessage(role, text, isError = false) {
  const node = document.createElement("div");
  node.className = `message ${role}${isError ? " error" : ""}`;
  const speaker = document.createElement("span");
  speaker.className = "message-speaker";
  speaker.textContent = role === "user" ? "You" : "Clawd";
  const body = document.createElement("p");
  body.className = "message-text";
  body.textContent = text;
  node.append(speaker, body);
  messagesEl.appendChild(node);
  messagesEl.scrollTop = messagesEl.scrollHeight;
  return node;
}

function announce(text, isError = false) {
  const status = document.getElementById("chat-status");
  status.textContent = text;
  status.style.color = isError ? "#ffd0c8" : "#aab8b2";
}

function showToast(text) {
  const toast = document.getElementById("toast");
  toast.textContent = text;
  toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { toast.hidden = true; }, 9_000);
}

function prettyTime(ms) {
  if (!ms) return "";
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(ms));
}

function renderFileResults(files) {
  const container = document.getElementById("file-results");
  container.replaceChildren();
  const matches = Array.isArray(files) ? files : [];
  if (!matches.length) {
    container.hidden = true;
    return;
  }
  container.hidden = false;
  for (const file of matches.slice(0, 3)) {
    const row = document.createElement("div");
    row.className = "file-result";
    const info = document.createElement("div");
    info.className = "file-info";
    const name = document.createElement("span");
    name.className = "file-name";
    name.textContent = file.name;
    const folder = document.createElement("span");
    folder.className = "file-folder";
    folder.textContent = file.folder_name;
    info.append(name, folder);
    const open = document.createElement("button");
    open.type = "button";
    open.className = "secondary-button";
    open.textContent = "Open location";
    open.addEventListener("click", async () => {
      open.disabled = true;
      try {
        await invoke("reveal_file", { path: file.path });
        showToast(`Showing ${file.name} in your file manager.`);
      } catch (error) {
        showToast(errorText(error));
      } finally {
        open.disabled = false;
      }
    });
    row.append(info, open);
    container.appendChild(row);
  }
  if (matches.length > 3) {
    const more = document.createElement("div");
    more.className = "file-more";
    more.textContent = `Showing 3 of ${matches.length} matches.`;
    container.appendChild(more);
  }
}

function errorText(error) {
  return typeof error === "string" ? error : (error && error.message) ? error.message : String(error || "Something went wrong.");
}

async function setPetAiWorking(active) {
  try { await invoke("set_assistant_working", { active }); } catch (_) {}
}

async function sendMessage(text) {
  const prompt = text.trim();
  if (!prompt || sendEl.disabled) return;
  const previousHistory = conversation.slice(-12).map((item) => ({ role: item.role, content: item.content }));
  conversation.push({ role: "user", content: prompt });
  messagesEl.replaceChildren();
  addMessage("user", prompt);
  const pendingMessage = addMessage("assistant", "Thinking…");
  pendingMessage.classList.add("pending");
  const fileResults = document.getElementById("file-results");
  fileResults.replaceChildren();
  fileResults.hidden = true;
  promptEl.value = "";
  promptEl.style.height = "auto";
  composerEl.hidden = true;
  askAgainEl.hidden = true;
  sendEl.disabled = true;
  sendEl.textContent = "…";
  announce("Clawd is thinking…");
  try {
    await setPetAiWorking(true);
    const reply = await invoke("assistant_chat", { prompt, history: previousHistory });
    pendingMessage.remove();
    addMessage("assistant", reply.text || "Done.");
    conversation.push({ role: "assistant", content: reply.text || "Done." });
    renderFileResults(reply.files);
    if (reply.todos && reply.todos.length) showToast(reply.text || "Added to your task list.");
    announce("");
  } catch (error) {
    const message = errorText(error);
    pendingMessage.remove();
    addMessage("assistant", message, true);
    announce("");
  } finally {
    await setPetAiWorking(false);
    sendEl.disabled = false;
    sendEl.textContent = "Send";
    composerEl.hidden = true;
    askAgainEl.hidden = false;
  }
}

document.getElementById("chat-form").addEventListener("submit", (event) => {
  event.preventDefault();
  sendMessage(promptEl.value);
});
askAgainEl.addEventListener("click", () => {
  askAgainEl.hidden = true;
  composerEl.hidden = false;
  promptEl.focus();
});
promptEl.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    document.getElementById("chat-form").requestSubmit();
  }
});
promptEl.addEventListener("input", () => {
  promptEl.style.height = "auto";
  promptEl.style.height = `${Math.min(promptEl.scrollHeight, 84)}px`;
});

document.getElementById("tasks-tab").addEventListener("click", () => showView("tasks-view"));
document.getElementById("settings-tab").addEventListener("click", () => showView("settings-view"));
document.querySelectorAll("[data-view]").forEach((button) => button.addEventListener("click", () => showView(button.dataset.view)));
document.getElementById("close-window").addEventListener("click", async () => {
  try {
    await invoke("set_assistant_hover", { source: "assistant", inside: false });
    const current = tauri.webviewWindow && tauri.webviewWindow.getCurrentWebviewWindow ? tauri.webviewWindow.getCurrentWebviewWindow() : null;
    if (current) await current.hide();
  } catch (_) {}
});

async function refreshTodos() {
  const list = document.getElementById("todo-list");
  list.replaceChildren();
  try {
    const todos = await invoke("list_todos");
    if (!todos.length) {
      const empty = document.createElement("li");
      empty.className = "muted";
      empty.textContent = "No tasks yet. Add one here or ask Clawd in chat.";
      list.appendChild(empty);
      return;
    }
    for (const todo of todos) {
      const item = document.createElement("li");
      item.className = "todo-item";
      const check = document.createElement("input");
      check.type = "checkbox";
      check.checked = todo.completed;
      check.setAttribute("aria-label", `Mark ${todo.title} complete`);
      check.addEventListener("change", async () => {
        try { await invoke("complete_todo", { id: todo.id, completed: check.checked }); refreshTodos(); }
        catch (error) { showToast(errorText(error)); }
      });
      const copy = document.createElement("div");
      copy.className = "todo-copy";
      const title = document.createElement("div");
      title.className = `todo-title${todo.completed ? " done" : ""}`;
      title.textContent = todo.title;
      copy.appendChild(title);
      if (todo.due_at_ms) {
        const due = document.createElement("div");
        due.className = "todo-due";
        due.textContent = `${todo.reminder_notified ? "Reminder sent" : "Reminder"} · ${prettyTime(todo.due_at_ms)}`;
        copy.appendChild(due);
      }
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "delete-todo";
      remove.textContent = "×";
      remove.setAttribute("aria-label", `Delete ${todo.title}`);
      remove.addEventListener("click", async () => {
        try { await invoke("delete_todo", { id: todo.id }); refreshTodos(); }
        catch (error) { showToast(errorText(error)); }
      });
      item.append(check, copy, remove);
      list.appendChild(item);
    }
  } catch (error) {
    const row = document.createElement("li");
    row.className = "muted";
    row.textContent = errorText(error);
    list.appendChild(row);
  }
}

document.getElementById("todo-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const input = document.getElementById("todo-title");
  const title = input.value.trim();
  if (!title) return;
  try {
    await invoke("add_todo", { title });
    input.value = "";
    await refreshTodos();
  } catch (error) { showToast(errorText(error)); }
});

async function refreshSettings() {
  const status = document.getElementById("settings-status");
  status.textContent = "";
  try {
    const settings = await invoke("assistant_settings");
    document.getElementById("model-name").value = settings.model || "gemma4:31b";
    const keyStatus = document.getElementById("key-status");
    const keyState = settings.api_key_status || (settings.api_key_configured ? "saved" : settings.api_key_issue ? "unavailable" : "missing");
    const keyLabels = {
      saved: "Key saved",
      missing: "Not connected",
      invalid: "Saved key invalid",
      unavailable: "Credential store unavailable",
    };
    keyStatus.classList.toggle("key-status-good", keyState === "saved");
    keyStatus.classList.toggle("key-status-warning", keyState === "invalid" || keyState === "unavailable");
    keyStatus.textContent = keyLabels[keyState] || "Key status unknown";
    if (settings.api_key_issue) status.textContent = settings.api_key_issue;
    const startupToggle = document.getElementById("startup-toggle");
    startupToggle.checked = await invoke("autostart_enabled");
    const folderRoot = document.getElementById("folder-options");
    folderRoot.replaceChildren();
    for (const folder of settings.folders || []) {
      const label = document.createElement("label");
      label.className = "folder-option";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = folder.enabled;
      checkbox.disabled = !folder.exists;
      checkbox.value = folder.id;
      const text = document.createElement("span");
      text.textContent = folder.name;
      const path = document.createElement("span");
      path.className = "folder-path";
      path.textContent = folder.exists ? folder.path : "Folder not available";
      const copy = document.createElement("span");
      copy.append(text, path);
      label.append(checkbox, copy);
      checkbox.addEventListener("change", () => saveFolderPermissions());
      folderRoot.appendChild(label);
    }
  } catch (error) {
    status.textContent = errorText(error);
  }
}

async function saveFolderPermissions() {
  const status = document.getElementById("settings-status");
  const folderIds = Array.from(document.querySelectorAll("#folder-options input:checked")).map((input) => input.value);
  try {
    await invoke("set_enabled_folders", { folderIds });
    status.textContent = "Folder permissions updated.";
  } catch (error) { status.textContent = errorText(error); }
}

document.getElementById("save-model").addEventListener("click", async () => {
  const model = document.getElementById("model-name").value.trim();
  const status = document.getElementById("settings-status");
  try {
    await invoke("set_ollama_model", { model });
    status.textContent = "Model saved.";
  } catch (error) { status.textContent = errorText(error); }
});

document.getElementById("save-key").addEventListener("click", async () => {
  const input = document.getElementById("api-key");
  const status = document.getElementById("settings-status");
  if (!input.value.trim()) { status.textContent = "Paste an Ollama Cloud API key first."; return; }
  try {
    await invoke("save_ollama_api_key", { apiKey: input.value });
    input.value = "";
    await refreshSettings();
    status.textContent = "API key saved and verified in the operating system credential store.";
  } catch (error) {
    const message = errorText(error);
    await refreshSettings();
    status.textContent = message;
  }
});

document.getElementById("clear-key").addEventListener("click", async () => {
  const status = document.getElementById("settings-status");
  try {
    await invoke("clear_ollama_api_key");
    await refreshSettings();
    status.textContent = "Saved API key cleared.";
  } catch (error) { status.textContent = errorText(error); }
});

document.getElementById("startup-toggle").addEventListener("change", async (event) => {
  const toggle = event.currentTarget;
  const requested = toggle.checked;
  const status = document.getElementById("settings-status");
  toggle.disabled = true;
  try {
    toggle.checked = await invoke("set_autostart", { enabled: requested });
    status.textContent = toggle.checked
      ? "Clawd will open automatically when you sign in."
      : "Clawd will not start automatically.";
  } catch (error) {
    toggle.checked = !requested;
    status.textContent = errorText(error);
  } finally {
    toggle.disabled = false;
  }
});

async function refreshAll() {
  try {
    const todos = await invoke("list_todos");
    const due = todos.filter((todo) => todo.due_at_ms && todo.due_at_ms <= Date.now() && !todo.completed && !todo.reminder_notified);
    if (due.length) showToast(`Reminder: ${due[0].title}`);
  } catch (_) {}
}

async function setHover(inside) {
  try { await invoke("set_assistant_hover", { source: "assistant", inside }); } catch (_) {}
}
document.body.addEventListener("mouseenter", () => setHover(true));
document.body.addEventListener("mouseleave", () => setHover(false));

listen("reminder-due", (event) => {
  const task = event && event.payload;
  if (!task) return;
  showToast(`Reminder: ${task.title}`);
  if (!document.getElementById("tasks-view").hidden) refreshTodos();
}).catch(() => {});

refreshAll();
