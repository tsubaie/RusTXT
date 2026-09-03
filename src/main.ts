import { Compartment, EditorSelection, EditorState, type Extension } from "@codemirror/state";
import { EditorView, drawSelection, keymap, type ViewUpdate } from "@codemirror/view";
import {
  defaultKeymap,
  history,
  historyKeymap,
  redo,
  redoDepth,
  selectAll,
  undo,
  undoDepth,
} from "@codemirror/commands";
import {
  findNext,
  findPrevious,
  gotoLine,
  openSearchPanel,
  search,
  searchKeymap,
} from "@codemirror/search";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, message, open, save } from "@tauri-apps/plugin-dialog";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { api, type ClosedDocument, type Config, type DocumentState, type ResolvedTheme, type Settings } from "./api";
import { FindPanel, currentFindPanel } from "./find-panel";
import { MenuBar, type MenuEntry } from "./menu";
import { SettingsPage, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP } from "./settings-page";
import "./styles.css";

// ---------------------------------------------------------------------------
// Shell

const isMac = navigator.platform.startsWith("Mac") || navigator.userAgent.includes("Mac OS");
const isWindows = navigator.userAgent.includes("Windows");
const MOD = isMac ? "⌘" : "Ctrl";
const shortcut = (key: string, ...modifiers: string[]) => [MOD, ...modifiers, key].join("+");

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <div class="tabstrip" data-tauri-drag-region>
    <div id="tabs" class="tabs" role="tablist" aria-label="Open documents"></div>
    <button id="new-tab" class="new-tab" type="button" title="New tab (${shortcut("N")})" aria-label="New tab">+</button>
  </div>
  <div id="menubar" class="menubar"></div>
  <main id="editor" aria-label="Text editor"></main>
  <footer id="status" class="statusbar">
    <span id="status-position">Ln 1, Col 1</span>
    <span class="statusbar-right">
      <span id="status-count">0 characters</span>
      <span id="status-zoom">100%</span>
      <span id="status-eol">Unix (LF)</span>
      <span id="status-encoding">UTF-8</span>
    </span>
  </footer>
  <pre id="print-area" aria-hidden="true"></pre>
`;

const tabsHost = document.querySelector<HTMLElement>("#tabs")!;
const menubarHost = document.querySelector<HTMLElement>("#menubar")!;
const editorHost = document.querySelector<HTMLElement>("#editor")!;
const statusBar = document.querySelector<HTMLElement>("#status")!;
const statusPosition = document.querySelector<HTMLElement>("#status-position")!;
const statusCount = document.querySelector<HTMLElement>("#status-count")!;
const statusZoom = document.querySelector<HTMLElement>("#status-zoom")!;
const statusEol = document.querySelector<HTMLElement>("#status-eol")!;
const printArea = document.querySelector<HTMLElement>("#print-area")!;

// ---------------------------------------------------------------------------
// State

let documents: DocumentState[] = [];
let activeId = "";
/** One EditorState per open document so each tab keeps its own undo history. */
const states = new Map<string, EditorState>();
/** Content as last saved or loaded, for accurate dirty tracking. */
const savedContent = new Map<string, string>();
/** Documents whose content changed since the last snapshot write. */
const pendingSnapshots = new Set<string>();
let closedDocuments: ClosedDocument[] = [];
let persistTimer: number | undefined;
const wordWrap = new Compartment();

/** Everything user-configurable, as read from ~/.config/rustpad/config.toml. */
let settings: Settings = {
  config: {
    appearance: { theme: "auto", zoom: 100 },
    editor: { word_wrap: true },
    window: { status_bar: true, title_bar: "auto" },
  },
  theme: { requested: "auto", source: "system", mode: "system", palette: null, note: null },
  customThemes: [],
  configPath: "~/.config/rustpad/config.toml",
  omarchyAvailable: false,
  tilingCompositor: false,
  decorated: true,
  configError: null,
};

const active = () => documents.find((doc) => doc.id === activeId);
const order = () => documents.map((doc) => doc.id);

async function report(error: unknown) {
  const text = error instanceof Error ? error.message : String(error);
  console.error(error);
  try {
    await message(text, { title: "RustPad", kind: "error" });
  } catch {
    /* dialog unavailable; already logged */
  }
}

// ---------------------------------------------------------------------------
// Editor

const editor = new EditorView({
  parent: editorHost,
  state: EditorState.create({ extensions: baseExtensions() }),
});
editor.scrollDOM.addEventListener("scroll", () => schedulePersist(), { passive: true });

function baseExtensions(): Extension {
  return [
    history(),
    drawSelection(),
    search({ top: true, createPanel: (view) => new FindPanel(view) }),
    keymap.of([...editorKeymap(), ...searchKeymap, ...historyKeymap, ...defaultKeymap]),
    wordWrap.of(settings.config.editor.word_wrap ? EditorView.lineWrapping : []),
    EditorView.contentAttributes.of({ spellcheck: "false", autocorrect: "off", autocapitalize: "off" }),
    EditorView.updateListener.of(onEditorUpdate),
  ];
}

function editorKeymap() {
  return [
    { key: "Mod-h", run: () => (openReplace(), true) },
    { key: "Mod-g", run: gotoLine },
    { key: "F5", run: () => (insertTimeDate(), true) },
  ];
}

function createState(doc: DocumentState) {
  return EditorState.create({
    doc: doc.content,
    selection: EditorSelection.cursor(Math.min(doc.cursorOffset, doc.content.length)),
    extensions: baseExtensions(),
  });
}

function onEditorUpdate(update: ViewUpdate) {
  if (!update.docChanged && !update.selectionSet) return;
  const doc = active();
  if (!doc) return;
  doc.cursorOffset = update.state.selection.main.head;
  if (update.docChanged) {
    doc.content = update.state.doc.toString();
    const saved = savedContent.get(doc.id);
    const dirty = saved === undefined ? true : doc.content !== saved;
    if (dirty !== doc.dirty) {
      doc.dirty = dirty;
      renderTabs();
    }
    pendingSnapshots.add(doc.id);
  }
  updateStatus();
  schedulePersist();
}

function activate(id: string) {
  const next = documents.find((doc) => doc.id === id);
  if (!next) return;
  const previous = active();
  if (previous && previous.id !== id) {
    states.set(previous.id, editor.state);
    previous.scrollTop = editor.scrollDOM.scrollTop;
  }
  activeId = id;
  const state = states.get(id) ?? createState(next);
  states.set(id, state);
  editor.setState(state);
  applyWordWrap();
  requestAnimationFrame(() => {
    editor.scrollDOM.scrollTop = next.scrollTop;
  });
  closeSettings();
  renderTabs();
  updateStatus();
  editor.focus();
  void api.updateLayout(order(), activeId).catch(report);
}

// ---------------------------------------------------------------------------
// Persistence

function schedulePersist() {
  clearTimeout(persistTimer);
  persistTimer = window.setTimeout(() => void flush(), 400);
}

/** Write pending snapshots (content) and the active view state (cursor/scroll). */
async function flush() {
  clearTimeout(persistTimer);
  const doc = active();
  if (doc) doc.scrollTop = editor.scrollDOM.scrollTop;
  const pending = [...pendingSnapshots];
  pendingSnapshots.clear();
  try {
    for (const id of pending) {
      const target = documents.find((item) => item.id === id);
      if (target) await api.saveSnapshot(target);
    }
    if (doc && !pending.includes(doc.id)) await api.saveViewState(doc.id, doc.cursorOffset, doc.scrollTop);
  } catch (error) {
    pending.forEach((id) => pendingSnapshots.add(id));
    await report(error);
  }
}

async function refreshClosed() {
  try {
    closedDocuments = await api.listClosedDocuments();
  } catch (error) {
    console.error(error);
  }
}

// ---------------------------------------------------------------------------
// Documents

function freshDocument(): DocumentState {
  const used = new Set(
    documents.map((doc) => Number(/^Untitled (\d+)$/.exec(doc.title)?.[1] ?? NaN)).filter(Number.isFinite),
  );
  let number = 1;
  while (used.has(number)) number += 1;
  return {
    id: crypto.randomUUID(),
    filePath: null,
    title: `Untitled ${number}`,
    content: "",
    dirty: false,
    cursorOffset: 0,
    scrollTop: 0,
    tabPosition: documents.length,
    lineEnding: isWindows ? "CRLF" : "LF",
  };
}

async function addDocument(doc = freshDocument()) {
  doc.tabPosition = documents.length;
  documents.push(doc);
  if (!doc.dirty) savedContent.set(doc.id, doc.content);
  try {
    await api.saveSnapshot(doc); // the row must exist before the layout references it
  } catch (error) {
    await report(error);
  }
  activate(doc.id);
}

async function closeTab(id: string, discard: boolean) {
  const doc = documents.find((item) => item.id === id);
  if (!doc) return;
  if (discard && doc.dirty) {
    const confirmed = await ask(`Permanently discard unsaved changes to "${doc.title}"?`, {
      title: "RustPad",
      kind: "warning",
      okLabel: "Discard",
      cancelLabel: "Cancel",
    });
    if (!confirmed) return;
  }
  try {
    if (!discard && pendingSnapshots.has(id)) await api.saveSnapshot(doc);
    pendingSnapshots.delete(id);
    await api.closeDocument(id, discard);
  } catch (error) {
    await report(error);
  }
  const index = documents.findIndex((item) => item.id === id);
  documents = documents.filter((item) => item.id !== id);
  documents.forEach((item, position) => (item.tabPosition = position));
  states.delete(id);
  savedContent.delete(id);
  if (!documents.length) await addDocument();
  else activate(documents[Math.min(index, documents.length - 1)].id);
  await refreshClosed();
}

async function closeOtherTabs(keepId: string) {
  for (const doc of documents.filter((item) => item.id !== keepId)) await closeTab(doc.id, false);
}

async function openFile() {
  const selected = await open({ multiple: false, directory: false });
  if (!selected) return;
  try {
    const doc = await api.openDocument(selected);
    const existing = documents.find((item) => item.filePath === doc.filePath);
    if (existing) activate(existing.id);
    else await addDocument(doc);
  } catch (error) {
    await report(error);
  }
}

async function saveFile(forcePicker = false, target = active()) {
  if (!target) return false;
  let path = target.filePath;
  if (!path || forcePicker) {
    path = await save({
      defaultPath: target.filePath ?? `${target.title}.txt`,
      filters: [
        { name: "Text documents", extensions: ["txt"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
  }
  if (!path) return false;
  try {
    const saved = await api.saveDocument(target, path);
    Object.assign(target, saved);
    savedContent.set(target.id, target.content);
    pendingSnapshots.delete(target.id);
    renderTabs();
    updateStatus();
    return true;
  } catch (error) {
    await report(error);
    return false;
  }
}

async function saveAll() {
  for (const doc of documents.filter((item) => item.dirty)) {
    if (!doc.filePath) activate(doc.id);
    if (!(await saveFile(false, doc))) return;
  }
}

async function reopenClosed(id = closedDocuments[0]?.id) {
  if (!id) return;
  try {
    const doc = await api.reopenDocument(id);
    const existing = doc.filePath ? documents.find((item) => item.filePath === doc.filePath) : undefined;
    if (existing) {
      activate(existing.id); // the layout update re-closes the duplicate row
    } else {
      documents.push(doc);
      doc.tabPosition = documents.length - 1;
      if (!doc.dirty) savedContent.set(doc.id, doc.content);
      activate(doc.id);
    }
  } catch (error) {
    await report(error);
  }
  await refreshClosed();
}

async function exitApp() {
  clearTimeout(persistTimer);
  try {
    await flush();
    if (activeId) await api.updateLayout(order(), activeId);
  } catch (error) {
    console.error(error);
  } finally {
    await getCurrentWindow().destroy();
  }
}

// ---------------------------------------------------------------------------
// Edit actions

async function copySelection(cut: boolean) {
  const selection = editor.state.selection.main;
  if (selection.empty) return;
  try {
    await writeText(editor.state.sliceDoc(selection.from, selection.to));
    if (cut) {
      editor.dispatch({
        changes: { from: selection.from, to: selection.to, insert: "" },
        selection: { anchor: selection.from },
        userEvent: "delete.cut",
      });
    }
  } catch (error) {
    await report(error);
  }
  editor.focus();
}

async function pasteClipboard() {
  try {
    const text = await readText();
    if (text) editor.dispatch({ ...editor.state.replaceSelection(text), scrollIntoView: true, userEvent: "input.paste" });
  } catch (error) {
    await report(error);
  }
  editor.focus();
}

function deleteSelection() {
  const selection = editor.state.selection.main;
  if (!selection.empty) editor.dispatch({ changes: { from: selection.from, to: selection.to }, userEvent: "delete" });
  editor.focus();
}

function openFind() {
  closeSettings();
  openSearchPanel(editor);
}

function openReplace() {
  closeSettings();
  openSearchPanel(editor);
  currentFindPanel()?.showReplace(true);
}

function insertTimeDate() {
  const now = new Date();
  const stamp = `${now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })} ${now.toLocaleDateString()}`;
  editor.dispatch({ ...editor.state.replaceSelection(stamp), scrollIntoView: true, userEvent: "input" });
  editor.focus();
}

function printDocument() {
  printArea.textContent = active()?.content ?? "";
  window.print();
}

// ---------------------------------------------------------------------------
// Settings: config.toml is the source of truth; this only applies it.

const settingsPage = new SettingsPage({
  settings: () => settings,
  update: updateConfig,
  close: closeSettings,
});
editorHost.after(settingsPage.dom);

async function refreshSettings() {
  try {
    applySettings(await api.getSettings());
  } catch (error) {
    await report(error);
  }
}

async function updateConfig(mutate: (config: Config) => void) {
  const next: Config = structuredClone(settings.config);
  mutate(next);
  try {
    applySettings(await api.updateConfig(next));
  } catch (error) {
    await report(error);
  }
}

function applySettings(next: Settings) {
  settings = next;
  applyTheme(settings.theme);
  applyZoom();
  applyWordWrap();
  statusBar.hidden = !settings.config.window.status_bar;
  document.documentElement.classList.toggle("undecorated", !settings.decorated);
  settingsPage.refresh();
}

/** Built-in looks come from the stylesheet; palettes override its tokens. */
function applyTheme(theme: ResolvedTheme) {
  const root = document.documentElement;
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const mode = theme.mode === "system" ? (systemDark ? "dark" : "light") : theme.mode;
  root.dataset.theme = mode;
  const tokens = [
    "--surface", "--text", "--chrome", "--menu-bg", "--border", "--muted", "--accent",
    "--selection", "--search-match", "--hover", "--pressed", "--scrollbar",
  ];
  tokens.forEach((token) => root.style.removeProperty(token));
  const palette = theme.palette;
  if (!palette) return;
  const dark = mode === "dark";
  const bg = palette.background;
  const fg = palette.foreground;
  const accent = palette.accent ?? (dark ? "#4cc2ff" : "#005fb8");
  const set = (token: string, value: string) => root.style.setProperty(token, value);
  set("--surface", bg);
  set("--text", fg);
  set("--accent", accent);
  set("--chrome", palette.chrome ?? `color-mix(in srgb, ${bg} ${dark ? 82 : 95}%, black)`);
  set("--menu-bg", palette.menu ?? `color-mix(in srgb, ${bg} 92%, ${fg})`);
  set("--border", palette.border ?? `color-mix(in srgb, ${bg} 85%, ${fg})`);
  set("--muted", palette.muted ?? `color-mix(in srgb, ${fg} 60%, ${bg})`);
  set("--selection", palette.selection ?? `color-mix(in srgb, ${accent} 35%, transparent)`);
  set("--search-match", `color-mix(in srgb, ${accent} 30%, transparent)`);
  set("--hover", `color-mix(in srgb, ${fg} 7%, transparent)`);
  set("--pressed", `color-mix(in srgb, ${fg} 4%, transparent)`);
  set("--scrollbar", `color-mix(in srgb, ${fg} 30%, transparent)`);
}

function applyZoom() {
  document.documentElement.style.setProperty("--zoom", String(settings.config.appearance.zoom / 100));
  statusZoom.textContent = `${settings.config.appearance.zoom}%`;
}

function setZoom(zoom: number) {
  const clamped = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(zoom / ZOOM_STEP) * ZOOM_STEP));
  if (clamped === settings.config.appearance.zoom) return;
  void updateConfig((config) => (config.appearance.zoom = clamped));
}

function applyWordWrap() {
  editor.dispatch({ effects: wordWrap.reconfigure(settings.config.editor.word_wrap ? EditorView.lineWrapping : []) });
}

function openSettings() {
  menubar.close();
  editorHost.hidden = true;
  settingsPage.open();
}

function closeSettings() {
  if (!settingsPage.isOpen) return;
  settingsPage.close();
  editorHost.hidden = false;
  editor.focus();
}

function toggleSettings() {
  if (settingsPage.isOpen) closeSettings();
  else openSettings();
}

// ---------------------------------------------------------------------------
// Rendering

function renderTabs() {
  const current = active();
  if (current) void getCurrentWindow().setTitle(`${current.dirty ? "*" : ""}${current.title} - RustPad`).catch(console.error);
  tabsHost.replaceChildren(
    ...documents.map((doc) => {
      const tab = document.createElement("div");
      tab.className = `tab${doc.id === activeId ? " active" : ""}${doc.dirty ? " dirty" : ""}`;
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-selected", String(doc.id === activeId));
      tab.title = doc.filePath ?? doc.title;

      const title = document.createElement("button");
      title.type = "button";
      title.className = "tab-title";
      title.textContent = doc.title;
      title.addEventListener("click", () => activate(doc.id));

      const dirty = document.createElement("span");
      dirty.className = "tab-dirty";
      dirty.textContent = doc.dirty ? "●" : "";

      const close = document.createElement("button");
      close.type = "button";
      close.className = "tab-close";
      close.title = `Close tab (${shortcut("W")})`;
      close.setAttribute("aria-label", `Close ${doc.title}`);
      close.textContent = "×";
      close.addEventListener("click", (event) => {
        event.stopPropagation();
        void closeTab(doc.id, false);
      });

      tab.append(title, dirty, close);
      tab.addEventListener("mousedown", (event) => {
        if (event.button === 1) {
          event.preventDefault();
          void closeTab(doc.id, false);
        }
      });
      tab.addEventListener("contextmenu", (event) => {
        event.preventDefault();
        menubar.openContextMenu(tabContextMenu(doc), event.clientX, event.clientY);
      });
      return tab;
    }),
  );
  tabsHost.querySelector(".tab.active")?.scrollIntoView({ inline: "nearest", block: "nearest" });
}

tabsHost.addEventListener(
  "wheel",
  (event) => {
    if (!event.deltaY || event.ctrlKey || event.metaKey) return;
    event.preventDefault();
    tabsHost.scrollLeft += event.deltaY;
  },
  { passive: false },
);

function updateStatus() {
  const doc = active();
  if (!doc) {
    statusPosition.textContent = "Ready";
    return;
  }
  const head = editor.state.selection.main.head;
  const line = editor.state.doc.lineAt(head);
  statusPosition.textContent = `Ln ${line.number}, Col ${head - line.from + 1}`;
  statusCount.textContent = `${editor.state.doc.length.toLocaleString()} characters`;
  statusEol.textContent = doc.lineEnding === "CRLF" ? "Windows (CRLF)" : "Unix (LF)";
}

// ---------------------------------------------------------------------------
// Menus

const hasSelection = () => !editor.state.selection.main.empty;
const closedLabel = (doc: ClosedDocument) => `${doc.title}${doc.dirty ? " ●" : ""}`;

function recentlyClosedMenu(): MenuEntry[] {
  if (!closedDocuments.length) return [{ label: "Nothing to reopen", enabled: () => false }];
  return closedDocuments.map((doc) => ({ label: closedLabel(doc), action: () => void reopenClosed(doc.id) }));
}

function tabContextMenu(doc: DocumentState): MenuEntry[] {
  return [
    { label: "Close tab", shortcut: shortcut("W"), action: () => void closeTab(doc.id, false) },
    { label: "Close other tabs", enabled: () => documents.length > 1, action: () => void closeOtherTabs(doc.id) },
    { label: "Discard changes and close", action: () => void closeTab(doc.id, true) },
    { kind: "separator" },
    { label: "Save", shortcut: shortcut("S"), action: () => void saveFile(false, doc) },
    { label: "Save as…", shortcut: shortcut("S", "Shift"), action: () => void saveFile(true, doc) },
  ];
}

const menubar = new MenuBar(
  menubarHost,
  [
    {
      label: "File",
      mnemonic: "f",
      items: () => [
        { label: "New tab", shortcut: shortcut("N"), action: () => void addDocument() },
        { label: "Open…", shortcut: shortcut("O"), action: () => void openFile() },
        { label: "Recently closed", submenu: recentlyClosedMenu },
        { kind: "separator" },
        { label: "Save", shortcut: shortcut("S"), action: () => void saveFile() },
        { label: "Save as…", shortcut: shortcut("S", "Shift"), action: () => void saveFile(true) },
        {
          label: "Save all",
          shortcut: shortcut("S", isMac ? "Option" : "Alt"),
          enabled: () => documents.some((doc) => doc.dirty),
          action: () => void saveAll(),
        },
        { kind: "separator" },
        { label: "Print…", shortcut: shortcut("P"), action: printDocument },
        { kind: "separator" },
        { label: "Close tab", shortcut: shortcut("W"), action: () => void closeTab(activeId, false) },
        { label: "Discard changes and close", action: () => void closeTab(activeId, true) },
        {
          label: "Reopen closed tab",
          shortcut: shortcut("T", "Shift"),
          enabled: () => closedDocuments.length > 0,
          action: () => void reopenClosed(),
        },
        { kind: "separator" },
        { label: "Settings", shortcut: shortcut(","), action: openSettings },
        { label: "Exit", shortcut: shortcut("W", "Shift"), action: () => void exitApp() },
      ],
    },
    {
      label: "Edit",
      mnemonic: "e",
      items: () => [
        { label: "Undo", shortcut: shortcut("Z"), enabled: () => undoDepth(editor.state) > 0, action: () => (undo(editor), editor.focus()) },
        { label: "Redo", shortcut: shortcut("Y"), enabled: () => redoDepth(editor.state) > 0, action: () => (redo(editor), editor.focus()) },
        { kind: "separator" },
        { label: "Cut", shortcut: shortcut("X"), enabled: hasSelection, action: () => void copySelection(true) },
        { label: "Copy", shortcut: shortcut("C"), enabled: hasSelection, action: () => void copySelection(false) },
        { label: "Paste", shortcut: shortcut("V"), action: () => void pasteClipboard() },
        { label: "Delete", shortcut: "Del", enabled: hasSelection, action: deleteSelection },
        { kind: "separator" },
        { label: "Find…", shortcut: shortcut("F"), action: openFind },
        { label: "Find next", shortcut: "F3", action: () => (findNext(editor), editor.focus()) },
        { label: "Find previous", shortcut: "Shift+F3", action: () => (findPrevious(editor), editor.focus()) },
        { label: "Replace…", shortcut: shortcut("H"), action: openReplace },
        { label: "Go to…", shortcut: shortcut("G"), action: () => gotoLine(editor) },
        { kind: "separator" },
        { label: "Select all", shortcut: shortcut("A"), action: () => (selectAll(editor), editor.focus()) },
        { label: "Time/Date", shortcut: "F5", action: insertTimeDate },
      ],
    },
    {
      label: "View",
      mnemonic: "v",
      items: () => [
        {
          label: "Zoom",
          submenu: () => [
            { label: "Zoom in", shortcut: shortcut("+"), action: () => setZoom(settings.config.appearance.zoom + ZOOM_STEP) },
            { label: "Zoom out", shortcut: shortcut("-"), action: () => setZoom(settings.config.appearance.zoom - ZOOM_STEP) },
            { label: "Restore default zoom", shortcut: shortcut("0"), action: () => setZoom(100) },
          ],
        },
        {
          label: "Status bar",
          checked: () => settings.config.window.status_bar,
          action: () => void updateConfig((config) => (config.window.status_bar = !config.window.status_bar)),
        },
        {
          label: "Word wrap",
          checked: () => settings.config.editor.word_wrap,
          action: () => void updateConfig((config) => (config.editor.word_wrap = !config.editor.word_wrap)),
        },
        { kind: "separator" },
        { label: "Settings", shortcut: shortcut(","), action: openSettings },
      ],
    },
  ],
  () => editor.focus(),
);

// The gear on the right opens the Settings page, as in Notepad.
const gear = document.createElement("button");
gear.type = "button";
gear.className = "menubar-item menubar-icon";
gear.title = `Settings (${shortcut(",")})`;
gear.setAttribute("aria-label", "Settings");
gear.textContent = "⚙";
gear.addEventListener("click", toggleSettings);
menubarHost.append(gear);

// ---------------------------------------------------------------------------
// Global shortcuts and wiring

document.querySelector("#new-tab")!.addEventListener("click", () => void addDocument());

window.addEventListener("keydown", (event) => {
  if (menubar.isOpen) return;
  if (event.key === "Escape" && settingsPage.isOpen) {
    event.preventDefault();
    closeSettings();
    return;
  }
  if (event.altKey && !event.ctrlKey && !event.metaKey && event.key.length === 1) {
    if (menubar.openByMnemonic(event.key)) event.preventDefault();
    return;
  }
  if (event.key === "F5") {
    // CodeMirror handles F5 when focused; this covers focus elsewhere.
    if (!event.defaultPrevented) insertTimeDate();
    event.preventDefault();
    return;
  }
  const mod = isMac ? event.metaKey : event.ctrlKey;
  if (!mod) return;
  const key = event.key.toLowerCase();
  const run = (action: () => unknown) => {
    event.preventDefault();
    void action();
  };
  if (event.altKey) {
    if (key === "s") run(saveAll);
    return;
  }
  switch (key) {
    case "n": return run(() => addDocument());
    case "o": return run(openFile);
    case "s": return run(() => saveFile(event.shiftKey));
    case "w": return run(() => (event.shiftKey ? exitApp() : closeTab(activeId, false)));
    case "t": if (event.shiftKey) run(reopenClosed); return;
    case "p": return run(printDocument);
    case ",": return run(toggleSettings);
    case "h": if (!event.defaultPrevented) run(openReplace); return;
    case "g": if (!event.defaultPrevented) run(() => gotoLine(editor)); return;
    case "f": if (!event.defaultPrevented) run(openFind); return;
    case "=": case "+": return run(() => setZoom(settings.config.appearance.zoom + ZOOM_STEP));
    case "-": return run(() => setZoom(settings.config.appearance.zoom - ZOOM_STEP));
    case "0": return run(() => setZoom(100));
    case "tab": {
      if (documents.length < 2) return;
      event.preventDefault();
      const index = documents.findIndex((doc) => doc.id === activeId);
      const step = event.shiftKey ? -1 : 1;
      activate(documents[(index + step + documents.length) % documents.length].id);
      return;
    }
  }
});

window.addEventListener(
  "wheel",
  (event) => {
    if (!(isMac ? event.metaKey : event.ctrlKey)) return;
    event.preventDefault();
    setZoom(settings.config.appearance.zoom + (event.deltaY < 0 ? ZOOM_STEP : -ZOOM_STEP));
  },
  { passive: false },
);

window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => applyTheme(settings.theme));
await listen("settings-changed", () => void refreshSettings());

await getCurrentWindow().onCloseRequested(async (event) => {
  event.preventDefault();
  await exitApp();
});

// ---------------------------------------------------------------------------
// Startup

await refreshSettings();

try {
  const session = await api.restoreSession();
  documents = session.documents;
  documents.forEach((doc, index) => {
    doc.tabPosition = index;
    if (!doc.dirty) savedContent.set(doc.id, doc.content);
  });
  if (!documents.length) await addDocument();
  else activate(session.activeId && documents.some((doc) => doc.id === session.activeId) ? session.activeId : documents[0].id);
} catch (error) {
  await report(error);
  if (!documents.length) await addDocument();
}
await refreshClosed();
