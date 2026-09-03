/**
 * The Settings page, in the style of Windows 11 Notepad: it replaces the
 * editor area with grouped cards of labelled rows, and a back arrow returns
 * to the document. Every control writes straight to config.toml through the
 * host, so the file on disk is always the source of truth.
 */

import type { Config, Settings, TitlebarMode } from "./api";

export type SettingsHost = {
  settings: () => Settings;
  update: (mutate: (config: Config) => void) => Promise<void>;
  close: () => void;
};

export const ZOOM_STEP = 10;
export const ZOOM_MIN = 10;
export const ZOOM_MAX = 500;

export class SettingsPage {
  readonly dom: HTMLElement;

  constructor(private readonly host: SettingsHost) {
    this.dom = document.createElement("section");
    this.dom.id = "settings";
    this.dom.className = "settings";
    this.dom.setAttribute("aria-label", "Settings");
    this.dom.hidden = true;
  }

  get isOpen() {
    return !this.dom.hidden;
  }

  open() {
    this.render();
    this.dom.hidden = false;
    this.dom.querySelector<HTMLElement>(".settings-back")?.focus();
  }

  close() {
    this.dom.hidden = true;
  }

  /** Re-render if visible; called whenever settings change. */
  refresh() {
    if (this.isOpen) this.render();
  }

  private render() {
    const settings = this.host.settings();
    const { config, theme } = settings;
    this.dom.replaceChildren();

    const header = el("header", "settings-header");
    const back = el("button", "settings-back") as HTMLButtonElement;
    back.type = "button";
    back.title = "Back (Esc)";
    back.setAttribute("aria-label", "Back to document");
    back.textContent = "←";
    back.addEventListener("click", () => this.host.close());
    const title = el("h1", "settings-title");
    title.textContent = "Settings";
    header.append(back, title);

    const body = el("div", "settings-body");

    // Appearance ---------------------------------------------------------
    const appearance = card("Appearance");
    appearance.append(
      row(
        "App theme",
        this.themeDescription(settings),
        select(
          this.themeOptions(settings),
          config.appearance.theme,
          (value) => void this.host.update((c) => (c.appearance.theme = value)),
        ),
      ),
      row(
        "Window title bar",
        settings.tilingCompositor
          ? "Automatic hides it here because a tiling compositor is running"
          : "Automatic keeps the native title bar on this desktop",
        select(
          [
            { value: "auto", label: "Automatic" },
            { value: "show", label: "Always show" },
            { value: "hide", label: "Always hide" },
          ],
          config.window.title_bar,
          (value) => void this.host.update((c) => (c.window.title_bar = value as TitlebarMode)),
        ),
      ),
    );

    // Text -----------------------------------------------------------------
    const text = card("Text");
    text.append(
      row("Zoom", "Also Ctrl + mouse wheel, Ctrl + plus and Ctrl + minus", this.zoomControl(config)),
      row(
        "Word wrap",
        "Wrap long lines to the window width",
        toggle(config.editor.word_wrap, (on) => void this.host.update((c) => (c.editor.word_wrap = on))),
      ),
      row(
        "Status bar",
        "Line and column, character count, zoom, line endings and encoding",
        toggle(config.window.status_bar, (on) => void this.host.update((c) => (c.window.status_bar = on))),
      ),
    );

    // Configuration --------------------------------------------------------
    const files = card("Configuration files");
    const note = el("p", "settings-note");
    note.append(
      "Settings are stored in ",
      code(settings.configPath),
      ". Edits made there, and Omarchy theme changes, apply immediately. Custom themes are TOML files in ",
      code(settings.configPath.replace(/config\.toml$/, "themes/")),
      " with mode, background, foreground and optional accent, muted, selection, border, chrome and menu colors.",
    );
    files.append(note);
    if (settings.configError) files.append(warning(settings.configError));
    if (theme.note) files.append(warning(theme.note));

    // About ------------------------------------------------------------------
    const about = card("About");
    const version = el("p", "settings-note");
    version.textContent = "RustPad 0.1.0. A fast, recoverable, cross-platform text editor.";
    about.append(version);

    body.append(appearance, text, files, about);
    this.dom.append(header, body);
  }

  private themeOptions(settings: Settings) {
    const options = [
      { value: "auto", label: settings.omarchyAvailable ? "Automatic (Omarchy theme)" : "Automatic (system setting)" },
      { value: "system", label: "Use system setting" },
      { value: "light", label: "Light" },
      { value: "dark", label: "Dark" },
    ];
    if (settings.omarchyAvailable) options.push({ value: "omarchy", label: "Follow Omarchy theme" });
    for (const name of settings.customThemes) options.push({ value: name, label: `Custom: ${name}` });
    const current = settings.config.appearance.theme;
    if (!options.some((option) => option.value === current)) {
      options.push({ value: current, label: `${current} (not found)` });
    }
    return options;
  }

  private themeDescription(settings: Settings) {
    switch (settings.theme.source) {
      case "omarchy":
        return "Following the active Omarchy theme";
      case "custom":
        return `Using ~/.config/rustpad/themes/${settings.theme.requested}.toml`;
      case "fallback":
        return "Falling back to the system setting";
      default:
        return "Light and dark built-in looks, or follow the desktop";
    }
  }

  private zoomControl(config: Config) {
    const group = el("div", "settings-stepper");
    const minus = stepButton("−", "Zoom out");
    const value = el("span", "settings-stepper-value");
    value.textContent = `${config.appearance.zoom}%`;
    const plus = stepButton("+", "Zoom in");
    const reset = el("button", "settings-link") as HTMLButtonElement;
    reset.type = "button";
    reset.textContent = "Reset";
    reset.disabled = config.appearance.zoom === 100;
    const set = (zoom: number) =>
      void this.host.update((c) => (c.appearance.zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom))));
    minus.addEventListener("click", () => set(config.appearance.zoom - ZOOM_STEP));
    plus.addEventListener("click", () => set(config.appearance.zoom + ZOOM_STEP));
    reset.addEventListener("click", () => set(100));
    group.append(minus, value, plus, reset);
    return group;
  }
}

function el(tag: string, className: string) {
  const node = document.createElement(tag);
  node.className = className;
  return node;
}

function code(text: string) {
  const node = document.createElement("code");
  node.textContent = text;
  return node;
}

function card(heading: string) {
  const section = el("section", "settings-card");
  const title = el("h2", "settings-card-title");
  title.textContent = heading;
  section.append(title);
  return section;
}

function row(label: string, description: string, control: HTMLElement) {
  const line = el("div", "settings-row");
  const text = el("div", "settings-row-text");
  const name = el("div", "settings-row-label");
  name.textContent = label;
  const hint = el("div", "settings-row-hint");
  hint.textContent = description;
  text.append(name, hint);
  const id = `setting-${label.toLowerCase().replace(/\W+/g, "-")}`;
  control.id = id;
  name.setAttribute("for", id);
  control.setAttribute("aria-label", label);
  line.append(text, control);
  return line;
}

function select(options: { value: string; label: string }[], current: string, onChange: (value: string) => void) {
  const node = document.createElement("select");
  node.className = "settings-select";
  for (const option of options) {
    const item = document.createElement("option");
    item.value = option.value;
    item.textContent = option.label;
    item.selected = option.value === current;
    node.append(item);
  }
  node.addEventListener("change", () => onChange(node.value));
  return node;
}

function toggle(on: boolean, onChange: (on: boolean) => void) {
  const node = document.createElement("button");
  node.type = "button";
  node.className = "settings-toggle";
  node.setAttribute("role", "switch");
  node.setAttribute("aria-checked", String(on));
  const knob = el("span", "settings-toggle-knob");
  const state = el("span", "settings-toggle-text");
  state.textContent = on ? "On" : "Off";
  node.append(knob, state);
  node.addEventListener("click", () => onChange(node.getAttribute("aria-checked") !== "true"));
  return node;
}

function stepButton(label: string, title: string) {
  const node = el("button", "settings-step") as HTMLButtonElement;
  node.type = "button";
  node.textContent = label;
  node.title = title;
  node.setAttribute("aria-label", title);
  return node;
}

function warning(text: string) {
  const node = el("p", "settings-warning");
  node.setAttribute("role", "alert");
  node.textContent = text;
  return node;
}
