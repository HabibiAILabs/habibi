import { renderMarkdown } from "/assets/markdown.js";

const MARKDOWN_KEYS = new Set(["content", "description", "error", "instructions", "message", "preview", "text"]);

export function humanizeKey(key) {
  return String(key).replaceAll("_", " ").replace(/\b\w/g, letter => letter.toUpperCase());
}

export function renderRecord(value, { omitKeys = [] } = {}) {
  const omitted = new Set(omitKeys);
  return renderValue(value, null, omitted);
}

function renderValue(value, key, omitted) {
  if (value === null || value === undefined) return text("—", "record-empty");
  if (Array.isArray(value)) {
    if (!value.length) return text("None", "record-empty");
    const list = document.createElement("ol");
    list.className = "record-array";
    for (const item of value) {
      const row = document.createElement("li");
      row.append(renderValue(item, key, omitted));
      list.append(row);
    }
    return list;
  }
  if (typeof value === "object") {
    const fields = document.createElement("dl");
    fields.className = "record-fields";
    const entries = Object.entries(value).filter(([name, item]) => !omitted.has(name) && item !== undefined);
    if (!entries.length) return text("None", "record-empty");
    for (const [name, item] of entries) {
      const term = document.createElement("dt");
      term.textContent = humanizeKey(name);
      const detail = document.createElement("dd");
      detail.append(renderValue(item, name, omitted));
      fields.append(term, detail);
    }
    return fields;
  }
  if (typeof value === "boolean") return text(value ? "Yes" : "No");
  if (typeof value === "number") return text(value.toLocaleString());
  const string = String(value);
  if (MARKDOWN_KEYS.has(key) && (string.includes("\n") || string.length > 100)) {
    const rendered = renderMarkdown(string);
    rendered.classList.add("record-markdown");
    return rendered;
  }
  return text(string, looksLikeIdentifier(string) ? "record-code" : "");
}

function text(value, className = "") {
  const element = document.createElement("span");
  if (className) element.className = className;
  element.textContent = value;
  return element;
}

function looksLikeIdentifier(value) {
  return /^([0-9a-f]{8}-|01[a-z0-9]{6,}|sha256:|[a-f0-9]{32,}$)/i.test(value);
}
