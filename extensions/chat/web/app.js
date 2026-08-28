const api = "./api";
const sessionsElement = document.querySelector("#sessions");
const messagesElement = document.querySelector("#messages");
const titleElement = document.querySelector("#session-title");
const statusElement = document.querySelector("#status");
const composer = document.querySelector("#composer");
const input = document.querySelector("#message-input");
const newSessionButton = document.querySelector("#new-session");

let sessions = [];
let activeSessionId = null;
let busy = false;

async function request(path, options = {}) {
  const response = await fetch(`${api}${path}`, {
    ...options,
    headers: { "content-type": "application/json", ...(options.headers || {}) },
  });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || `Request failed (${response.status})`);
  return result;
}

function setBusy(value, label = "thinking") {
  busy = value;
  statusElement.textContent = value ? label : "ready";
  statusElement.classList.toggle("busy", value);
  input.disabled = value;
  composer.querySelector("button").disabled = value;
}

async function loadSessions(preferredId) {
  sessions = (await request("/sessions")).filter((session) => !session.archived);
  if (sessions.length === 0) {
    const created = await request("/sessions", {
      method: "POST",
      body: JSON.stringify({ title: "New chat" }),
    });
    sessions = await request("/sessions");
    preferredId = created.id;
  }
  activeSessionId = preferredId || activeSessionId || sessions[0].id;
  renderSessions();
  await loadMessages();
}

function renderSessions() {
  sessionsElement.replaceChildren(...sessions.map((session) => {
    const button = document.createElement("button");
    button.className = `session${session.id === activeSessionId ? " active" : ""}`;
    button.textContent = session.title;
    button.onclick = async () => {
      activeSessionId = session.id;
      renderSessions();
      await loadMessages();
    };
    return button;
  }));
  const active = sessions.find((session) => session.id === activeSessionId);
  titleElement.textContent = active?.title || "Chat";
}

async function loadMessages() {
  const messages = await request(`/sessions/${activeSessionId}/messages`);
  if (messages.length === 0) {
    const empty = document.createElement("p");
    empty.className = "empty";
    empty.textContent = "Start anywhere. Habibi's memory is larger than this session.";
    messagesElement.replaceChildren(empty);
    return;
  }
  messagesElement.replaceChildren(...messages.map(renderMessage));
  messagesElement.scrollTop = messagesElement.scrollHeight;
}

function renderMessage(message) {
  const article = document.createElement("article");
  article.className = `message ${message.role}`;
  const role = document.createElement("div");
  role.className = "role";
  role.textContent = message.role === "assistant" ? "Habibi" : "You";
  const content = document.createElement("div");
  content.className = "content";
  content.textContent = message.content;
  article.append(role, content);
  return article;
}

newSessionButton.onclick = async () => {
  const created = await request("/sessions", {
    method: "POST",
    body: JSON.stringify({ title: "New chat" }),
  });
  await loadSessions(created.id);
  input.focus();
};

composer.addEventListener("submit", async (event) => {
  event.preventDefault();
  const content = input.value.trim();
  if (!content || busy || !activeSessionId) return;
  input.value = "";
  setBusy(true);
  let failure = null;
  try {
    const result = await request(`/sessions/${activeSessionId}/messages`, {
      method: "POST",
      body: JSON.stringify({ content }),
    });
    await loadSessions(activeSessionId);
    if (result.reaction_error) throw new Error(`Message saved, but the reaction failed: ${result.reaction_error}`);
  } catch (error) {
    failure = error;
    console.error(error);
  } finally {
    setBusy(false);
    if (failure) statusElement.textContent = failure.message;
    input.focus();
  }
});

input.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    composer.requestSubmit();
  }
});

loadSessions().catch((error) => {
  statusElement.textContent = error.message;
  console.error(error);
});
