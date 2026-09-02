const id = decodeURIComponent(location.pathname.split("/").filter(Boolean)[1] || "");
const heading = document.querySelector("#app-name");
const icon = document.querySelector("#app-icon");
const status = document.querySelector("#app-status");
const frame = document.querySelector("#app-frame");

function initials(name) {
  return name.split(/\s+/).map(part => part[0]).join("").slice(0, 2).toUpperCase();
}

try {
  const response = await fetch("/api/extensions");
  if (!response.ok) throw new Error(`Could not load extension (${response.status})`);
  const extensions = await response.json();
  const extension = extensions.find(value => value.id === id);
  if (!extension || !extension.enabled || !extension.frame_page) throw new Error("Extension app is unavailable");
  const appName = extension.app_name || extension.name;
  document.title = `${appName} · Habibi`;
  heading.textContent = appName;
  if (extension.icon) {
    const image = document.createElement("img");
    image.src = extension.icon;
    image.alt = "";
    icon.append(image);
  } else {
    icon.textContent = initials(appName);
  }
  frame.src = extension.frame_page;
  frame.title = `${appName} extension`;
  frame.hidden = false;
  status.textContent = "";
} catch (error) {
  heading.textContent = "Unavailable";
  status.textContent = error.message;
}
