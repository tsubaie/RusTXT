import { invoke } from "@tauri-apps/api/core";

export type LineEnding = "LF" | "CRLF";

export type DocumentState = {
  id: string;
  filePath: string | null;
  title: string;
  /** Always uses `\n`; `lineEnding` records how the file is written. */
  content: string;
  dirty: boolean;
  cursorOffset: number;
  scrollTop: number;
  tabPosition: number;
  lineEnding: LineEnding;
};

export type ClosedDocument = {
  id: string;
  title: string;
  filePath: string | null;
  dirty: boolean;
  closedAt: number;
};

export type Session = { documents: DocumentState[]; activeId: string | null };

export type TitlebarMode = "auto" | "show" | "hide";

/** Mirrors ~/.config/rustpad/config.toml, so keys are snake_case like the file. */
export type Config = {
  appearance: { theme: string; zoom: number };
  editor: { word_wrap: boolean };
  window: { status_bar: boolean; title_bar: TitlebarMode };
};

export type Palette = {
  mode: string;
  background: string;
  chrome: string | null;
  foreground: string;
  muted: string | null;
  accent: string | null;
  selection: string | null;
  border: string | null;
  menu: string | null;
};

export type ResolvedTheme = {
  requested: string;
  source: "system" | "light" | "dark" | "omarchy" | "custom" | "fallback";
  mode: "light" | "dark" | "system";
  palette: Palette | null;
  note: string | null;
};

export type Settings = {
  config: Config;
  theme: ResolvedTheme;
  customThemes: string[];
  configPath: string;
  omarchyAvailable: boolean;
  tilingCompositor: boolean;
  decorated: boolean;
  configError: string | null;
};

/** Typed wrappers over the Rust command boundary. */
export const api = {
  restoreSession: () => invoke<Session>("restore_session"),
  saveSnapshot: (document: DocumentState) => invoke<void>("save_snapshot", { document }),
  saveViewState: (id: string, cursorOffset: number, scrollTop: number) =>
    invoke<void>("save_view_state", { id, cursorOffset, scrollTop }),
  updateLayout: (order: string[], activeId: string) => invoke<void>("update_layout", { order, activeId }),
  closeDocument: (id: string, discard: boolean) => invoke<void>("close_document", { id, discard }),
  listClosedDocuments: () => invoke<ClosedDocument[]>("list_closed_documents"),
  reopenDocument: (id: string) => invoke<DocumentState>("reopen_document", { id }),
  openDocument: (path: string) => invoke<DocumentState>("open_document", { path }),
  saveDocument: (document: DocumentState, path: string) =>
    invoke<DocumentState>("save_document", { document, path }),
  getSettings: () => invoke<Settings>("get_settings"),
  updateConfig: (config: Config) => invoke<Settings>("update_config", { config }),
};
