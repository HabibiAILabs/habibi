const api = "./api";
const $ = (selector) => document.querySelector(selector);
const sessionsElement = $("#sessions");
const messagesElement = $("#messages");
const titleElement = $("#session-title");
const statusElement = $("#status");
const composer = $("#composer");
const input = $("#message-input");
const sendButton = $("#send-message");
const newSessionButton = $("#new-session");
const deleteSessionButton = $("#delete-session");
const renameTitleButton = $("#rename-title");
const searchInput = $("#session-search");
const sidebar = $("#sidebar");
const dialog = $("#session-dialog");
const dialogForm = $("#session-dialog-form");
const dialogTitle = $("#dialog-title");
const dialogDescription = $("#dialog-description");
const dialogNameLabel = $("#dialog-name-label");
const dialogName = $("#dialog-name");
const dialogConfirm = $("#dialog-confirm");
const toast = $("#toast");

const DRAFT_SESSION_ID = "__draft_session__";
let sessions = [];
let activeSessionId = null;
let draftTitle = "New conversation";
let messages = [];
let busy = false;
let dialogAction = null;
let toastTimer = null;

async function request(path, options = {}) {
  const response = await fetch(`${api}${path}`, {
    ...options,
    headers: { "content-type": "application/json", ...(options.headers || {}) },
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `Request failed (${response.status})`);
  return result;
}

function setBusy(value, label = "processing event…") {
  busy = value;
  statusElement.textContent = value ? label : "ready";
  statusElement.classList.toggle("busy", value);
  sendButton.disabled = value || !activeSessionId;
  newSessionButton.disabled = value;
  deleteSessionButton.disabled = value || !activeSessionId;
  renameTitleButton.disabled = value || !activeSessionId;
}

function setStatus(label, kind = "") {
  statusElement.textContent = label;
  statusElement.className = `status ${kind}`.trim();
}

function showToast(message) {
  clearTimeout(toastTimer);
  toast.textContent = message;
  toast.hidden = false;
  toastTimer = setTimeout(() => { toast.hidden = true; }, 4200);
}

async function loadSessions(preferredId) {
  sessions = (await request("/sessions")).filter((session) => !session.archived);
  const candidate = preferredId || activeSessionId;
  activeSessionId = candidate === DRAFT_SESSION_ID || sessions.some((session) => session.id === candidate)
    ? candidate
    : sessions[0]?.id || DRAFT_SESSION_ID;
  renderSessions();
  return loadMessages();
}

function renderSessions() {
  const needle = searchInput.value.trim().toLowerCase();
  const projectedSessions = activeSessionId === DRAFT_SESSION_ID
    ? [{ id: DRAFT_SESSION_ID, title: draftTitle, last_message: "Not saved yet", draft: true }, ...sessions]
    : sessions;
  const visible = projectedSessions.filter((session) => !needle || session.title.toLowerCase().includes(needle));
  sessionsElement.replaceChildren(...visible.map((session) => {
    const row = document.createElement("div");
    row.className = `session${session.id === activeSessionId ? " active" : ""}`;
    const button = document.createElement("button");
    button.type = "button";
    button.className = "session-main";
    const title = document.createElement("strong");
    title.textContent = session.title;
    const preview = document.createElement("span");
    preview.textContent = session.last_message || "No messages yet";
    button.append(title, preview);
    button.onclick = async () => {
      if (busy || session.id === activeSessionId) return;
      activeSessionId = session.id;
      renderSessions();
      await loadMessages();
      sidebar.classList.remove("open");
      input.focus();
    };
    const rename = document.createElement("button");
    rename.type = "button";
    rename.className = "session-action";
    rename.textContent = "•••";
    rename.title = `Rename ${session.title}`;
    rename.onclick = () => openRenameDialog(session);
    row.append(button, rename);
    return row;
  }));
  if (!visible.length) {
    const empty = document.createElement("p");
    empty.className = "session-empty";
    empty.textContent = sessions.length ? "No matching chats" : "No conversations yet";
    sessionsElement.append(empty);
  }
  const active = activeSessionId === DRAFT_SESSION_ID
    ? { id: DRAFT_SESSION_ID, title: draftTitle, draft: true }
    : sessions.find((session) => session.id === activeSessionId);
  titleElement.textContent = active?.title || "No conversation";
  input.disabled = !activeSessionId;
  input.placeholder = activeSessionId ? "Message Habibi…" : "Create a conversation to begin";
  setBusy(busy);
}

async function loadMessages() {
  if (activeSessionId === DRAFT_SESSION_ID) {
    messages = [];
    const empty = document.createElement("div");
    empty.className = "empty empty-state";
    empty.innerHTML = "<strong>Start anywhere</strong><span>This conversation will be saved when you send its first message.</span>";
    messagesElement.replaceChildren(empty);
    return messages;
  }
  if (!activeSessionId) {
    messages = [];
    const empty = document.createElement("div");
    empty.className = "empty empty-state";
    empty.innerHTML = "<strong>Start a conversation</strong><span>Name it, then begin anywhere.</span>";
    const button = document.createElement("button");
    button.className = "primary";
    button.textContent = "New conversation";
    button.onclick = startNewConversation;
    empty.append(button);
    messagesElement.replaceChildren(empty);
    return messages;
  }
  messages = await request(`/sessions/${activeSessionId}/messages`);
  renderMessages();
  return messages;
}

function renderMessages(extraMessage) {
  const displayed = extraMessage ? [...messages, extraMessage] : messages;
  if (!displayed.length) {
    const empty = document.createElement("div");
    empty.className = "empty empty-state";
    empty.innerHTML = "<strong>Start anywhere</strong><span>This session is a view; Habibi's event memory is larger.</span>";
    messagesElement.replaceChildren(empty);
    return;
  }
  messagesElement.replaceChildren(...displayed.map(renderMessage));
  requestAnimationFrame(() => { messagesElement.scrollTop = messagesElement.scrollHeight; });
}

function renderMessage(message) {
  const article = document.createElement("article");
  article.className = `message ${message.role}${message.pending ? " pending" : ""}`;
  const heading = document.createElement("div");
  heading.className = "message-heading";
  const role = document.createElement("span");
  role.className = "role";
  role.textContent = message.role === "assistant" ? "Habibi" : "You";
  heading.append(role);
  if (message.created_at) {
    const time = document.createElement("time");
    time.dateTime = message.created_at;
    time.textContent = new Date(message.created_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    heading.append(time);
  }
  const content = document.createElement("div");
  content.className = "content";
  content.textContent = message.content;
  article.append(heading, content);
  return article;
}

function startNewConversation() {
  if (busy) return;
  draftTitle = "New conversation";
  activeSessionId = DRAFT_SESSION_ID;
  searchInput.value = "";
  renderSessions();
  loadMessages();
  sidebar.classList.remove("open");
  input.focus();
}

function openRenameDialog(session = activeSessionId === DRAFT_SESSION_ID
  ? { id: DRAFT_SESSION_ID, title: draftTitle, draft: true }
  : sessions.find((item) => item.id === activeSessionId)) {
  if (!session || busy) return;
  dialogAction = "rename";
  dialog.dataset.sessionId = session.id;
  dialogTitle.textContent = "Rename conversation";
  dialogDescription.textContent = "Use a short, recognizable name for the sidebar.";
  dialogNameLabel.hidden = false;
  dialogName.value = session.title;
  dialogName.placeholder = "Conversation name";
  dialogConfirm.textContent = "Save name";
  dialogConfirm.className = "primary";
  dialog.showModal();
  setTimeout(() => { dialogName.focus(); dialogName.select(); }, 0);
}

function openDeleteDialog() {
  const session = activeSessionId === DRAFT_SESSION_ID
    ? { id: DRAFT_SESSION_ID, title: draftTitle, draft: true }
    : sessions.find((item) => item.id === activeSessionId);
  if (!session || busy) return;
  dialogAction = "delete";
  dialog.dataset.sessionId = session.id;
  dialogTitle.textContent = `Remove “${session.title}”?`;
  dialogDescription.textContent = session.draft
    ? "This conversation has not been saved yet, so there is no event history to preserve."
    : "This removes the session from the chat sidebar. Its immutable messages, events, and semantic event links remain available in Habibi's history.";
  dialogNameLabel.hidden = true;
  dialogConfirm.textContent = "Remove conversation";
  dialogConfirm.className = "danger";
  dialog.showModal();
  setTimeout(() => dialogConfirm.focus(), 0);
}

async function completeDialog() {
  const action = dialogAction;
  const sessionId = dialog.dataset.sessionId;
  const title = dialogName.value.trim();
  if (action === "rename" && !title) {
    dialogName.setCustomValidity("Enter a conversation name");
    dialogName.reportValidity();
    return;
  }
  dialogName.setCustomValidity("");
  dialogConfirm.disabled = true;
  try {
    if (action === "rename") {
      if (sessionId === DRAFT_SESSION_ID) {
        draftTitle = title;
        dialog.close();
        renderSessions();
      } else {
        await request(`/sessions/${sessionId}`, { method: "PATCH", body: JSON.stringify({ title }) });
        dialog.close();
        await loadSessions(sessionId);
      }
      showToast("Conversation renamed");
    } else if (action === "delete") {
      if (sessionId === DRAFT_SESSION_ID) {
        dialog.close();
        activeSessionId = sessions[0]?.id || DRAFT_SESSION_ID;
        draftTitle = "New conversation";
        renderSessions();
        await loadMessages();
        showToast("Unsaved conversation discarded");
      } else {
        await request(`/sessions/${sessionId}`, { method: "DELETE" });
        dialog.close();
        activeSessionId = null;
        await loadSessions();
        showToast("Conversation removed from the sidebar; event history was preserved");
      }
    }
  } catch (error) {
    dialogDescription.textContent = error.message;
    console.error(error);
  } finally {
    dialogConfirm.disabled = false;
  }
}

newSessionButton.onclick = startNewConversation;
renameTitleButton.onclick = () => openRenameDialog();
deleteSessionButton.onclick = openDeleteDialog;
$("#dialog-cancel").onclick = () => dialog.close();
dialogForm.addEventListener("submit", (event) => { event.preventDefault(); completeDialog(); });
searchInput.addEventListener("input", renderSessions);
$("#mobile-sidebar").onclick = () => sidebar.classList.toggle("open");

composer.addEventListener("submit", async (event) => {
  event.preventDefault();
  const content = input.value.trim();
  if (!content || busy || !activeSessionId) return;
  const sessionId = activeSessionId;
  const previousAssistantSequence = Math.max(0, ...messages.filter((message) => message.role === "assistant").map((message) => message.sequence || 0));
  input.value = "";
  resizeInput();
  renderMessages({ role: "user", content, pending: true, created_at: new Date().toISOString() });
  setBusy(true, "processing event…");
  let outcomeStatus = null;
  try {
    const result = sessionId === DRAFT_SESSION_ID
      ? await request("/sessions", {
          method: "POST",
          body: JSON.stringify({ title: draftTitle, first_message: content }),
        })
      : await request(`/sessions/${sessionId}/messages`, {
          method: "POST",
          body: JSON.stringify({ content }),
        });
    const persistedSessionId = sessionId === DRAFT_SESSION_ID ? result.id : sessionId;
    if (activeSessionId === sessionId) {
      activeSessionId = persistedSessionId;
      await loadSessions(persistedSessionId);
      const replied = messages.some((message) => message.role === "assistant" && message.sequence > previousAssistantSequence);
      outcomeStatus = [replied ? "replied" : "settled · no chat reply", replied ? "success" : "settled"];
    }
    if (result.reaction_error) throw new Error(`Message saved, but processing failed: ${result.reaction_error}`);
  } catch (error) {
    if (sessionId === DRAFT_SESSION_ID) input.value = content;
    resizeInput();
    await loadMessages().catch(() => {});
    outcomeStatus = ["processing failed", "error"];
    showToast(error.message);
    console.error(error);
  } finally {
    setBusy(false);
    if (outcomeStatus) setStatus(...outcomeStatus);
    input.focus();
  }
});

function resizeInput() {
  input.style.height = "auto";
  input.style.height = `${Math.min(input.scrollHeight, 180)}px`;
}
input.addEventListener("input", resizeInput);
input.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (!event.shiftKey || event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    composer.requestSubmit();
  }
});

document.addEventListener("keydown", (event) => {
  const command = event.ctrlKey || event.metaKey;
  if (command && event.key.toLowerCase() === "k") {
    event.preventDefault();
    sidebar.classList.add("open");
    searchInput.focus();
    searchInput.select();
  } else if (command && event.key.toLowerCase() === "n") {
    event.preventDefault();
    startNewConversation();
  } else if (event.key === "Escape" && !dialog.open && searchInput.value) {
    searchInput.value = "";
    renderSessions();
    input.focus();
  }
});

loadSessions().then(() => input.focus()).catch((error) => {
  setStatus("offline", "error");
  showToast(error.message);
  console.error(error);
});
