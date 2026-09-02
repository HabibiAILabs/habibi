const container = document.querySelector("#home-extension-actions");
const status = document.querySelector("#home-extension-status");

function initials(name) {
  return name.split(/\s+/).map(part => part[0]).join("").slice(0, 2).toUpperCase();
}

function appCard(extension) {
  const link = document.createElement("a");
  link.className = "home-extension-card";
  link.href = extension.main_page;

  const visual = document.createElement("span");
  visual.className = "extension-logo home-extension-logo";
  if (extension.icon) {
    const image = document.createElement("img");
    image.src = extension.icon;
    image.alt = "";
    visual.append(image);
  } else {
    visual.textContent = initials(extension.app_name || extension.name);
  }

  const text = document.createElement("span");
  const name = document.createElement("strong");
  name.textContent = extension.app_name || extension.name;
  const description = document.createElement("small");
  description.textContent = extension.description || "Open extension";
  text.append(name, description);
  link.append(visual, text);
  return link;
}

if (location.pathname === "/") {
  try {
    const response = await fetch("/api/extensions");
    if (!response.ok) throw new Error(`Could not load apps (${response.status})`);
    const extensions = await response.json();
    const apps = extensions.filter(extension => extension.enabled && extension.main_page);
    container.replaceChildren(...apps.map(appCard));
    status.textContent = apps.length ? "" : "Extensions with home pages will appear here.";
  } catch (error) {
    status.textContent = error.message;
  }
}
