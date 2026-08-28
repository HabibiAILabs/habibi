export function renderMarkdown(markdown) {
  const root = document.createElement("div");
  root.className = "markdown";
  const lines = String(markdown || "").replace(/\r\n?/g, "\n").split("\n");
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }

    if (line.startsWith("```")) {
      const language = line.slice(3).trim();
      const contents = [];
      index += 1;
      while (index < lines.length && !lines[index].startsWith("```")) {
        contents.push(lines[index]);
        index += 1;
      }
      if (index < lines.length) index += 1;
      const pre = document.createElement("pre");
      const code = document.createElement("code");
      if (language) code.dataset.language = language;
      code.textContent = contents.join("\n");
      pre.append(code);
      root.append(pre);
      continue;
    }

    const heading = line.match(/^(#{1,3})\s+(.+)$/);
    if (heading) {
      const element = document.createElement(`h${heading[1].length}`);
      appendInline(element, heading[2]);
      root.append(element);
      index += 1;
      continue;
    }

    if (/^[-*]\s+/.test(line)) {
      const list = document.createElement("ul");
      while (index < lines.length && /^[-*]\s+/.test(lines[index])) {
        const item = document.createElement("li");
        appendInline(item, lines[index].replace(/^[-*]\s+/, ""));
        list.append(item);
        index += 1;
      }
      root.append(list);
      continue;
    }

    if (/^\d+\.\s+/.test(line)) {
      const list = document.createElement("ol");
      while (index < lines.length && /^\d+\.\s+/.test(lines[index])) {
        const item = document.createElement("li");
        appendInline(item, lines[index].replace(/^\d+\.\s+/, ""));
        list.append(item);
        index += 1;
      }
      root.append(list);
      continue;
    }

    if (line.startsWith("> ")) {
      const quote = document.createElement("blockquote");
      const contents = [];
      while (index < lines.length && lines[index].startsWith("> ")) {
        contents.push(lines[index].slice(2));
        index += 1;
      }
      appendInline(quote, contents.join(" "));
      root.append(quote);
      continue;
    }

    const paragraph = [];
    while (
      index < lines.length
      && lines[index].trim()
      && !lines[index].startsWith("```")
      && !/^(#{1,3})\s+/.test(lines[index])
      && !/^[-*]\s+/.test(lines[index])
      && !/^\d+\.\s+/.test(lines[index])
      && !lines[index].startsWith("> ")
    ) {
      paragraph.push(lines[index]);
      index += 1;
    }
    const element = document.createElement("p");
    appendInline(element, paragraph.join("\n"));
    root.append(element);
  }
  return root;
}

function appendInline(parent, text) {
  const pattern = /(\*\*[^*\n]+\*\*|`[^`\n]+`|\[[^\]\n]+\]\([^\)\n]+\)|\*[^*\n]+\*)/g;
  let offset = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > offset) parent.append(document.createTextNode(text.slice(offset, match.index)));
    const token = match[0];
    if (token.startsWith("**")) {
      const strong = document.createElement("strong");
      strong.textContent = token.slice(2, -2);
      parent.append(strong);
    } else if (token.startsWith("`")) {
      const code = document.createElement("code");
      code.textContent = token.slice(1, -1);
      parent.append(code);
    } else if (token.startsWith("[")) {
      const parts = token.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
      const url = parts?.[2] || "";
      if (/^https?:\/\//i.test(url)) {
        const link = document.createElement("a");
        link.href = url;
        link.target = "_blank";
        link.rel = "noopener noreferrer";
        link.textContent = parts[1];
        parent.append(link);
      } else {
        parent.append(document.createTextNode(token));
      }
    } else {
      const emphasis = document.createElement("em");
      emphasis.textContent = token.slice(1, -1);
      parent.append(emphasis);
    }
    offset = match.index + token.length;
  }
  if (offset < text.length) parent.append(document.createTextNode(text.slice(offset)));
}
