/**
 * Compact find/replace panel in the style of Windows 11 Notepad: a small
 * floating toolbar over the top-right of the editor with an optional replace
 * row. It plugs into @codemirror/search so F3, Shift+F3, Escape and the
 * existing search commands keep working.
 */

import type { EditorView, Panel, ViewUpdate } from "@codemirror/view";
import { runScopeHandlers } from "@codemirror/view";
import {
  SearchQuery,
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  replaceAll,
  replaceNext,
  setSearchQuery,
} from "@codemirror/search";

const COUNT_LIMIT = 1000;

let current: FindPanel | null = null;
/** Where the user last dragged the panel, relative to the editor. */
let dragPosition: { left: number; top: number } | null = null;

/** The panel currently shown, if any. */
export const currentFindPanel = () => current;

export class FindPanel implements Panel {
  readonly dom: HTMLElement;
  readonly top = true;

  private query: SearchQuery;
  private readonly findField: HTMLInputElement;
  private readonly replaceField: HTMLInputElement;
  private readonly replaceRow: HTMLElement;
  private readonly toggleReplace: HTMLButtonElement;
  private readonly count: HTMLElement;
  private readonly caseToggle: HTMLButtonElement;
  private readonly wordToggle: HTMLButtonElement;
  private readonly regexpToggle: HTMLButtonElement;

  constructor(readonly view: EditorView) {
    this.query = getSearchQuery(view.state);
    current = this;

    this.dom = element("div", "find-panel");
    this.dom.setAttribute("role", "search");

    const findRow = element("div", "find-row");
    this.toggleReplace = iconButton("›", "Toggle replace (Ctrl+H)", "find-expander");
    this.toggleReplace.setAttribute("aria-expanded", "false");
    this.toggleReplace.addEventListener("click", () => this.showReplace(this.replaceRow.hidden));

    this.findField = element("input", "find-input") as HTMLInputElement;
    this.findField.placeholder = "Find";
    this.findField.name = "search";
    this.findField.setAttribute("main-field", "true");
    this.findField.setAttribute("aria-label", "Find");
    this.findField.value = this.query.search;
    this.findField.addEventListener("input", () => this.commit());

    this.count = element("span", "find-count");

    const previous = iconButton("↑", "Find previous (Shift+F3)");
    previous.addEventListener("click", () => findPrevious(view));
    const next = iconButton("↓", "Find next (F3)");
    next.addEventListener("click", () => findNext(view));

    this.caseToggle = optionButton("Aa", "Match case", this.query.caseSensitive, () => this.commit());
    this.wordToggle = optionButton("ab", "Match whole word", this.query.wholeWord, () => this.commit());
    this.regexpToggle = optionButton(".*", "Use regular expression", this.query.regexp, () => this.commit());

    const close = iconButton("×", "Close (Esc)", "find-close");
    close.addEventListener("click", () => closeSearchPanel(view));

    findRow.append(
      this.toggleReplace,
      this.findField,
      this.count,
      previous,
      next,
      this.caseToggle,
      this.wordToggle,
      this.regexpToggle,
      close,
    );

    this.replaceRow = element("div", "find-row replace-row");
    this.replaceRow.hidden = true;
    this.replaceField = element("input", "find-input") as HTMLInputElement;
    this.replaceField.placeholder = "Replace with";
    this.replaceField.name = "replace";
    this.replaceField.setAttribute("aria-label", "Replace with");
    this.replaceField.value = this.query.replace;
    this.replaceField.addEventListener("input", () => this.commit());
    const replaceOne = textButton("Replace", "Replace next match (Enter)");
    replaceOne.addEventListener("click", () => replaceNext(view));
    const replaceEvery = textButton("Replace all", "Replace every match");
    replaceEvery.addEventListener("click", () => replaceAll(view));
    this.replaceRow.append(this.replaceField, replaceOne, replaceEvery);

    this.dom.append(findRow, this.replaceRow);
    this.dom.addEventListener("keydown", (event) => this.onKeydown(event));
    this.dom.addEventListener("mousedown", (event) => this.startDrag(event));
    this.updateCount();
  }

  /** Behaves like a small tool window: drag it anywhere over the text. */
  private startDrag(event: MouseEvent) {
    if (event.button !== 0 || (event.target as HTMLElement).closest("input, button")) return;
    const container = this.dom.parentElement;
    if (!container) return;
    event.preventDefault();
    const bounds = () => this.view.dom.getBoundingClientRect();
    const rect = container.getBoundingClientRect();
    const offsetX = event.clientX - rect.left;
    const offsetY = event.clientY - rect.top;
    const move = (moveEvent: MouseEvent) => {
      const area = bounds();
      const left = Math.min(Math.max(0, moveEvent.clientX - offsetX - area.left), Math.max(0, area.width - rect.width));
      const top = Math.min(Math.max(0, moveEvent.clientY - offsetY - area.top), Math.max(0, area.height - rect.height));
      dragPosition = { left, top };
      this.applyPosition();
    };
    const stop = () => document.removeEventListener("mousemove", move);
    document.addEventListener("mousemove", move);
    document.addEventListener("mouseup", stop, { once: true });
  }

  private applyPosition() {
    const container = this.dom.parentElement;
    if (!container || !dragPosition) return;
    container.style.left = `${dragPosition.left}px`;
    container.style.top = `${dragPosition.top}px`;
    container.style.right = "auto";
  }

  showReplace(show: boolean) {
    this.replaceRow.hidden = !show;
    this.toggleReplace.setAttribute("aria-expanded", String(show));
    this.toggleReplace.classList.toggle("expanded", show);
    if (show) this.replaceField.focus();
  }

  mount() {
    this.applyPosition();
    this.findField.select();
  }

  destroy() {
    if (current === this) current = null;
  }

  update(update: ViewUpdate) {
    for (const transaction of update.transactions) {
      for (const effect of transaction.effects) {
        if (effect.is(setSearchQuery) && !effect.value.eq(this.query)) this.setQuery(effect.value);
      }
    }
    if (update.docChanged || update.selectionSet) this.updateCount();
  }

  private setQuery(query: SearchQuery) {
    this.query = query;
    this.findField.value = query.search;
    this.replaceField.value = query.replace;
    setPressed(this.caseToggle, query.caseSensitive);
    setPressed(this.wordToggle, query.wholeWord);
    setPressed(this.regexpToggle, query.regexp);
    this.updateCount();
  }

  private commit() {
    const query = new SearchQuery({
      search: this.findField.value,
      caseSensitive: isPressed(this.caseToggle),
      wholeWord: isPressed(this.wordToggle),
      regexp: isPressed(this.regexpToggle),
      replace: this.replaceField.value,
    });
    if (!query.eq(this.query)) {
      this.query = query;
      this.view.dispatch({ effects: setSearchQuery.of(query) });
    }
    this.updateCount();
  }

  private updateCount() {
    this.count.classList.remove("no-results");
    if (!this.query.search || !this.query.valid) {
      this.count.textContent = "";
      return;
    }
    const selection = this.view.state.selection.main;
    let total = 0;
    let index = 0;
    const cursor = this.query.getCursor(this.view.state);
    for (let match = cursor.next(); !match.done && total < COUNT_LIMIT; match = cursor.next()) {
      total += 1;
      if (match.value.from <= selection.from) index = total;
    }
    if (total === 0) {
      this.count.textContent = "No results";
      this.count.classList.add("no-results");
    } else {
      const shown = total >= COUNT_LIMIT ? `${COUNT_LIMIT}+` : String(total);
      this.count.textContent = index ? `${index} of ${shown}` : shown;
    }
  }

  private onKeydown(event: KeyboardEvent) {
    if (runScopeHandlers(this.view, event, "search-panel")) {
      event.preventDefault();
      return;
    }
    if (event.key !== "Enter") return;
    event.preventDefault();
    if (event.target === this.replaceField) replaceNext(this.view);
    else if (event.shiftKey) findPrevious(this.view);
    else findNext(this.view);
  }
}

function element(tag: string, className: string) {
  const node = document.createElement(tag);
  node.className = className;
  return node;
}

function iconButton(label: string, title: string, className = "") {
  const button = element("button", `find-button ${className}`.trim()) as HTMLButtonElement;
  button.type = "button";
  button.textContent = label;
  button.title = title;
  button.setAttribute("aria-label", title);
  return button;
}

function textButton(label: string, title: string) {
  const button = element("button", "find-button find-text-button") as HTMLButtonElement;
  button.type = "button";
  button.textContent = label;
  button.title = title;
  return button;
}

function optionButton(label: string, title: string, pressed: boolean, onChange: () => void) {
  const button = iconButton(label, title, "find-option");
  setPressed(button, pressed);
  button.addEventListener("click", () => {
    setPressed(button, !isPressed(button));
    onChange();
  });
  return button;
}

function setPressed(button: HTMLButtonElement, pressed: boolean) {
  button.setAttribute("aria-pressed", String(pressed));
}

function isPressed(button: HTMLButtonElement) {
  return button.getAttribute("aria-pressed") === "true";
}
