/**
 * A Windows 11 Notepad style menu bar: top-level buttons that open dropdown
 * popups, nested submenus, check marks, shortcut hints, hover switching
 * between menus, full keyboard navigation and Alt+mnemonic access.
 * The same machinery serves right-click context menus.
 */

export type MenuEntry =
  | { kind: "separator" }
  | {
      kind?: "item";
      label: string;
      shortcut?: string;
      action?: () => void;
      checked?: () => boolean;
      enabled?: () => boolean;
      submenu?: () => MenuEntry[];
    };

export type MenuDefinition = {
  label: string;
  /** Letter that opens the menu with Alt. */
  mnemonic?: string;
  /** Tooltip; also used as the accessible name when `icon` is set. */
  title?: string;
  /** Renders as an icon button pushed to the right end of the bar. */
  icon?: boolean;
  items: () => MenuEntry[];
};

const CONTEXT = -2;

export class MenuBar {
  private buttons: HTMLButtonElement[] = [];
  private openIndex = -1;
  /** Popup stack: index 0 is the root dropdown, higher indexes are submenus. */
  private layers: HTMLElement[] = [];
  private entryOf = new WeakMap<HTMLElement, MenuEntry>();
  private ownerOf = new WeakMap<HTMLElement, HTMLButtonElement>();

  constructor(
    private host: HTMLElement,
    private menus: MenuDefinition[],
    private onClosed: () => void,
  ) {
    host.setAttribute("role", "menubar");
    menus.forEach((menu, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = menu.icon ? "menubar-item menubar-icon" : "menubar-item";
      button.setAttribute("role", "menuitem");
      button.setAttribute("aria-haspopup", "true");
      button.setAttribute("aria-expanded", "false");
      if (menu.title) button.title = menu.title;
      if (menu.icon) button.setAttribute("aria-label", menu.title ?? menu.label);
      button.textContent = menu.label;
      button.addEventListener("mousedown", (event) => {
        event.preventDefault();
        if (this.openIndex === index) this.close();
        else this.open(index);
      });
      button.addEventListener("mouseenter", () => {
        if (this.openIndex >= 0 && this.openIndex !== index) this.open(index);
      });
      button.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " " || event.key === "ArrowDown") {
          event.preventDefault();
          this.open(index);
          this.focusItem(0, 0);
        }
      });
      host.append(button);
      this.buttons.push(button);
    });

    document.addEventListener("mousedown", (event) => {
      if (!this.isOpen) return;
      const target = event.target as Node;
      if (this.host.contains(target) || this.layers.some((layer) => layer.contains(target))) return;
      this.close();
    });
    window.addEventListener("blur", () => this.close());
    window.addEventListener("resize", () => this.close());
    document.addEventListener("keydown", (event) => this.onKeydown(event), true);
  }

  get isOpen() {
    return this.openIndex !== -1;
  }

  /** Open the menu whose mnemonic matches `key`. Returns false if none does. */
  openByMnemonic(key: string) {
    const index = this.menus.findIndex((menu) => menu.mnemonic === key.toLowerCase());
    if (index < 0) return false;
    this.open(index);
    this.focusItem(0, 0);
    return true;
  }

  /** Show `entries` as a context menu at viewport coordinates. */
  openContextMenu(entries: MenuEntry[], x: number, y: number) {
    this.closePopups();
    this.openIndex = CONTEXT;
    this.syncButtons();
    const popup = this.buildPopup(entries, 0);
    popup.style.left = `${x}px`;
    popup.style.top = `${y}px`;
    document.body.append(popup);
    this.layers.push(popup);
    this.clampToViewport(popup);
  }

  open(index: number) {
    this.closePopups();
    this.openIndex = index;
    this.syncButtons();
    const popup = this.buildPopup(this.menus[index].items(), 0);
    const rect = this.buttons[index].getBoundingClientRect();
    popup.style.left = `${rect.left}px`;
    popup.style.top = `${rect.bottom + 4}px`;
    document.body.append(popup);
    this.layers.push(popup);
    this.clampToViewport(popup);
  }

  close() {
    if (!this.isOpen) return;
    this.closePopups();
    this.openIndex = -1;
    this.syncButtons();
    this.onClosed();
  }

  private syncButtons() {
    this.buttons.forEach((button, index) => {
      const open = index === this.openIndex;
      button.classList.toggle("open", open);
      button.setAttribute("aria-expanded", String(open));
    });
  }

  private closePopups(fromLevel = 0) {
    while (this.layers.length > fromLevel) {
      const popup = this.layers.pop()!;
      this.ownerOf.get(popup)?.classList.remove("submenu-open");
      popup.remove();
    }
  }

  private buildPopup(entries: MenuEntry[], level: number) {
    const popup = document.createElement("div");
    popup.className = "menu-popup";
    popup.setAttribute("role", "menu");
    popup.tabIndex = -1;

    for (const entry of entries) {
      if (entry.kind === "separator") {
        const separator = document.createElement("div");
        separator.className = "menu-separator";
        separator.setAttribute("role", "separator");
        popup.append(separator);
        continue;
      }

      const item = document.createElement("button");
      item.type = "button";
      item.className = "menu-item";
      item.tabIndex = -1;
      const enabled = entry.enabled?.() ?? true;
      item.disabled = !enabled;
      this.entryOf.set(item, entry);

      const check = document.createElement("span");
      check.className = "menu-check";
      if (entry.checked) {
        item.setAttribute("role", "menuitemcheckbox");
        const checked = entry.checked();
        item.setAttribute("aria-checked", String(checked));
        check.textContent = checked ? "✓" : "";
      } else {
        item.setAttribute("role", "menuitem");
      }
      const label = document.createElement("span");
      label.className = "menu-label";
      label.textContent = entry.label;
      const shortcut = document.createElement("span");
      shortcut.className = "menu-shortcut";
      shortcut.textContent = entry.shortcut ?? "";
      const arrow = document.createElement("span");
      arrow.className = "menu-arrow";
      arrow.textContent = entry.submenu ? "›" : "";
      item.append(check, label, shortcut, arrow);

      if (entry.submenu) {
        item.setAttribute("aria-haspopup", "true");
        item.addEventListener("mouseenter", () => {
          item.focus();
          this.openSubmenu(item, entry, level);
        });
        item.addEventListener("click", () => this.openSubmenu(item, entry, level, true));
      } else {
        item.addEventListener("mouseenter", () => {
          this.closePopups(level + 1);
          if (enabled) item.focus();
        });
        item.addEventListener("click", () => {
          if (!enabled) return;
          this.close();
          entry.action?.();
        });
      }
      popup.append(item);
    }
    return popup;
  }

  private openSubmenu(item: HTMLButtonElement, entry: MenuEntry, level: number, focusFirst = false) {
    if (item.disabled || entry.kind === "separator" || !entry.submenu) return;
    const current = this.layers[level + 1];
    if (current && this.ownerOf.get(current) === item) {
      if (focusFirst) this.focusItem(level + 1, 0);
      return;
    }
    this.closePopups(level + 1);
    const submenu = this.buildPopup(entry.submenu(), level + 1);
    const rect = item.getBoundingClientRect();
    submenu.style.left = `${rect.right - 2}px`;
    submenu.style.top = `${rect.top - 6}px`;
    document.body.append(submenu);
    this.layers.push(submenu);
    this.ownerOf.set(submenu, item);
    item.classList.add("submenu-open");
    const bounds = submenu.getBoundingClientRect();
    if (bounds.right > window.innerWidth - 8) submenu.style.left = `${Math.max(8, rect.left - bounds.width + 2)}px`;
    this.clampToViewport(submenu);
    if (focusFirst) this.focusItem(level + 1, 0);
  }

  private clampToViewport(popup: HTMLElement) {
    const rect = popup.getBoundingClientRect();
    if (rect.right > window.innerWidth - 8) popup.style.left = `${Math.max(8, window.innerWidth - 8 - rect.width)}px`;
    if (rect.bottom > window.innerHeight - 8) popup.style.top = `${Math.max(8, window.innerHeight - 8 - rect.height)}px`;
  }

  private itemsOf(popup: HTMLElement) {
    return [...popup.querySelectorAll<HTMLButtonElement>(".menu-item:not(:disabled)")];
  }

  private focusItem(level: number, index: number) {
    const popup = this.layers[level];
    if (!popup) return;
    const items = this.itemsOf(popup);
    if (!items.length) return;
    items[((index % items.length) + items.length) % items.length].focus();
  }

  private onKeydown(event: KeyboardEvent) {
    if (!this.isOpen) return;
    const level = this.layers.length - 1;
    const popup = this.layers[level];
    const items = this.itemsOf(popup);
    const activeElement = document.activeElement as HTMLButtonElement | null;
    const focused = activeElement ? items.indexOf(activeElement) : -1;
    const menuCount = this.menus.length;
    const inBar = this.openIndex >= 0;

    // While a menu is open, nothing should leak through to the editor.
    event.stopPropagation();
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        if (level > 0) {
          const owner = this.ownerOf.get(popup);
          this.closePopups(level);
          owner?.focus();
        } else this.close();
        return;
      case "ArrowDown":
        event.preventDefault();
        this.focusItem(level, focused + 1);
        return;
      case "ArrowUp":
        event.preventDefault();
        this.focusItem(level, focused < 0 ? -1 : focused - 1);
        return;
      case "ArrowRight": {
        event.preventDefault();
        const entry = activeElement ? this.entryOf.get(activeElement) : undefined;
        if (entry && entry.kind !== "separator" && entry.submenu && activeElement) {
          this.openSubmenu(activeElement, entry, level, true);
        } else if (inBar) {
          this.open((this.openIndex + 1) % menuCount);
          this.focusItem(0, 0);
        }
        return;
      }
      case "ArrowLeft":
        event.preventDefault();
        if (level > 0) {
          const owner = this.ownerOf.get(popup);
          this.closePopups(level);
          owner?.focus();
        } else if (inBar) {
          this.open((this.openIndex - 1 + menuCount) % menuCount);
          this.focusItem(0, 0);
        }
        return;
      case "Enter":
      case " ":
        event.preventDefault();
        if (focused >= 0) activeElement!.click();
        else this.focusItem(level, 0);
        return;
      case "Tab":
        event.preventDefault();
        return;
      default:
        if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
          event.preventDefault();
          const letter = event.key.toLowerCase();
          const match = items.find((item) =>
            item.querySelector(".menu-label")?.textContent?.trim().toLowerCase().startsWith(letter),
          );
          if (match) {
            match.focus();
            match.click();
          }
        } else if (event.altKey && event.key.length === 1) {
          event.preventDefault();
          this.openByMnemonic(event.key);
        }
    }
  }
}
