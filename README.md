# RustPad

RustPad is a fast, recoverable, cross-platform text editor inspired by the simplicity of modern Windows Notepad.

The goal is to keep everyday text editing immediate and uncluttered while building a dependable foundation for session recovery, Markdown, large files, and optional writing assistance. RustPad targets Windows, macOS, and Linux from one codebase.

## Product principles

- Open quickly and stay out of the way.
- Never lose unsaved work.
- Make tabs and session restoration predictable.
- Keep saved files as ordinary files that remain usable everywhere.
- Use native operating-system behavior where it matters.
- Keep AI and cloud functionality optional.

## Planned stack

- **Desktop shell:** Tauri 2
- **Editor:** CodeMirror 6
- **Application core:** Rust
- **Recovery and session state:** SQLite through `rusqlite`, using WAL mode
- **Async work:** Tokio
- **File watching:** `notify`
- **Text encodings:** `encoding_rs`
- **Diagnostics:** `tracing`
- **Packaging and updates:** Tauri bundles and updater

The filesystem remains authoritative for saved documents. SQLite stores open tabs, unsaved snapshots, cursor and scroll positions, recent files, and crash-recovery metadata.

## Roadmap

### V1 — Reliable editing

Plain-text editing, multiple tabs, open/save, unsaved indicators, close-without-saving behavior, crash-safe recovery, and automatic session restoration.

### V2 — Everyday editor tools

Find and replace, encoding and line-ending controls, external-file change detection, recent files, themes, and configurable editor preferences.

### V3 — Rich text workflows

Markdown formatting and preview, spellcheck, printing, and export-oriented improvements.

### V4 — Power-user foundation

Large-file mode, command palette, advanced navigation, and a carefully permissioned extension model.

### V5 — Optional Writing Tools

Provider-neutral rewrite, summarize, and compose actions backed by local models or user-configured cloud providers.

Each roadmap version has a corresponding GitHub issue with its scope and acceptance criteria.

## Session behavior

RustPad deliberately distinguishes among these actions:

- **Close for now:** hide the tab while retaining its recovery state.
- **Discard changes:** explicitly and permanently remove the unsaved recovery snapshot.
- **Save:** atomically write the document to disk and update its recovery state.
- **Exit:** preserve all open tabs without forcing save prompts.

Recovery snapshots should be debounced during editing and flushed when focus changes or the application begins closing. Normal file saves should use a temporary file followed by an atomic replacement where supported.

## Architecture direction

The Rust application core should not depend directly on Tauri-specific types. The interface communicates with it through a small typed command and event boundary, allowing the core document, storage, search, and recovery services to be tested independently.

## Status

Planning. Implementation begins with V1.

