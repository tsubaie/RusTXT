import { EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import "./styles.css";

type DocumentState = {
  id: string;
  filePath: string | null;
  title: string;
  content: string;
  dirty: boolean;
  cursorOffset: number;
  scrollTop: number;
  tabPosition: number;
};

const app = document.querySelector<HTMLDivElement>("#app")!;
app.innerHTML = `
  <header class="toolbar">
    <div class="brand">RustPad</div>
    <button id="new" title="New (Ctrl+N)">New</button>
    <button id="open" title="Open (Ctrl+O)">Open</button>
    <button id="save" title="Save (Ctrl+S)">Save</button>
    <button id="save-as" title="Save As (Ctrl+Shift+S)">Save as</button>
    <span class="spacer"></span>
    <button id="theme" title="Toggle theme">◐</button>
  </header>
  <nav id="tabs" class="tabs" aria-label="Open documents"></nav>
  <main id="editor" aria-label="Text editor"></main>
  <footer id="status">Ready</footer>
`;

const tabs = document.querySelector<HTMLElement>("#tabs")!;
const editorHost = document.querySelector<HTMLElement>("#editor")!;
const status = document.querySelector<HTMLElement>("#status")!;
let documents: DocumentState[] = [];
let activeId = "";
let editor: EditorView | null = null;
let persistTimer: number | undefined;

const active = () => documents.find((doc) => doc.id === activeId);

function freshDocument(): DocumentState {
  const untitled = documents.filter((doc) => !doc.filePath).length + 1;
  return {
    id: crypto.randomUUID(), filePath: null, title: `Untitled ${untitled}`,
    content: "", dirty: false, cursorOffset: 0, scrollTop: 0,
    tabPosition: documents.length,
  };
}

function renderTabs() {
  tabs.replaceChildren(...documents.map((doc) => {
    const tab = document.createElement("div");
    tab.className = `tab ${doc.id === activeId ? "active" : ""}`;
    tab.setAttribute("role", "tab");
    tab.innerHTML = `<button class="tab-title">${escapeHtml(doc.title)}${doc.dirty ? " •" : ""}</button><button class="tab-close" title="Close for now">×</button><button class="tab-discard" title="Discard permanently">⌫</button>`;
    tab.querySelector<HTMLElement>(".tab-title")!.onclick = () => activate(doc.id);
    tab.querySelector<HTMLElement>(".tab-close")!.onclick = () => closeTab(doc.id, false);
    tab.querySelector<HTMLElement>(".tab-discard")!.onclick = () => closeTab(doc.id, true);
    return tab;
  }));
}

function escapeHtml(value: string) {
  const node = document.createElement("span"); node.textContent = value; return node.innerHTML;
}

function captureEditor() {
  const doc = active();
  if (!doc || !editor) return;
  doc.content = editor.state.doc.toString();
  doc.cursorOffset = editor.state.selection.main.head;
  doc.scrollTop = editor.scrollDOM.scrollTop;
}

function activate(id: string) {
  captureEditor();
  activeId = id;
  const doc = active();
  if (!doc) return;
  editor?.destroy();
  const selection = Math.min(doc.cursorOffset, doc.content.length);
  editor = new EditorView({
    parent: editorHost,
    state: EditorState.create({
      doc: doc.content,
      selection: { anchor: selection },
      extensions: [
        history(), keymap.of([...defaultKeymap, ...historyKeymap]),
        EditorView.lineWrapping,
        EditorView.updateListener.of((update) => {
          if (!update.docChanged && !update.selectionSet) return;
          const current = active();
          if (!current) return;
          current.content = update.state.doc.toString();
          current.cursorOffset = update.state.selection.main.head;
          if (update.docChanged) current.dirty = true;
          updateStatus(); renderTabs(); schedulePersist();
        }),
      ],
    }),
  });
  requestAnimationFrame(() => { if (editor) editor.scrollDOM.scrollTop = doc.scrollTop; });
  renderTabs(); updateStatus(); persistAll(); editor.focus();
}

function updateStatus() {
  const doc = active();
  if (!doc || !editor) { status.textContent = "Ready"; return; }
  const line = editor.state.doc.lineAt(editor.state.selection.main.head);
  status.textContent = `Ln ${line.number}, Col ${editor.state.selection.main.head - line.from + 1}  |  ${doc.content.length} characters  |  UTF-8`;
}

function schedulePersist() {
  clearTimeout(persistTimer);
  persistTimer = window.setTimeout(persistAll, 400);
}

async function persistAll() {
  captureEditor();
  await invoke("persist_session", { documents, activeId });
}

async function addDocument(doc = freshDocument()) {
  documents.push({ ...doc, tabPosition: documents.length });
  activate(doc.id);
}

async function closeTab(id: string, discard: boolean) {
  if (discard) {
    const doc = documents.find((item) => item.id === id);
    if (doc?.dirty && !confirm(`Permanently discard changes to ${doc.title}?`)) return;
  }
  await invoke("close_document", { id, discard });
  const index = documents.findIndex((doc) => doc.id === id);
  documents = documents.filter((doc) => doc.id !== id);
  documents.forEach((doc, position) => doc.tabPosition = position);
  if (!documents.length) documents.push(freshDocument());
  activate(documents[Math.min(index, documents.length - 1)].id);
}

async function openFile() {
  const selected = await open({ multiple: false, directory: false });
  if (!selected) return;
  const doc = await invoke<DocumentState>("open_document", { path: selected });
  const existing = documents.find((item) => item.filePath === doc.filePath);
  if (existing) activate(existing.id); else addDocument(doc);
}

async function saveFile(forcePicker = false) {
  captureEditor();
  const doc = active();
  if (!doc) return;
  let path = doc.filePath;
  if (!path || forcePicker) path = await save({ defaultPath: doc.filePath ?? `${doc.title}.txt` });
  if (!path) return;
  const saved = await invoke<DocumentState>("save_document", { document: doc, path });
  Object.assign(doc, saved); renderTabs(); updateStatus();
}

document.querySelector("#new")!.addEventListener("click", () => addDocument());
document.querySelector("#open")!.addEventListener("click", openFile);
document.querySelector("#save")!.addEventListener("click", () => saveFile());
document.querySelector("#save-as")!.addEventListener("click", () => saveFile(true));
document.querySelector("#theme")!.addEventListener("click", () => {
  document.documentElement.classList.toggle("light");
  localStorage.setItem("theme", document.documentElement.classList.contains("light") ? "light" : "dark");
});
window.addEventListener("keydown", (event) => {
  if (!event.ctrlKey) return;
  if (event.key.toLowerCase() === "n") { event.preventDefault(); addDocument(); }
  if (event.key.toLowerCase() === "o") { event.preventDefault(); openFile(); }
  if (event.key.toLowerCase() === "s") { event.preventDefault(); saveFile(event.shiftKey); }
  if (event.key === "Tab" && documents.length > 1) {
    event.preventDefault(); const index = documents.findIndex((doc) => doc.id === activeId);
    activate(documents[(index + (event.shiftKey ? -1 : 1) + documents.length) % documents.length].id);
  }
});
window.addEventListener("beforeunload", () => { void persistAll(); });
await getCurrentWindow().onCloseRequested(async (event) => {
  event.preventDefault();
  clearTimeout(persistTimer);
  await persistAll();
  await getCurrentWindow().destroy();
});
if (localStorage.getItem("theme") === "light") document.documentElement.classList.add("light");

const restored = await invoke<{ documents: DocumentState[]; activeId: string | null }>("restore_session");
documents = restored.documents;
if (!documents.length) documents = [freshDocument()];
activate(restored.activeId && documents.some((doc) => doc.id === restored.activeId) ? restored.activeId : documents[0].id);
