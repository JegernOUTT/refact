import type { Terminal } from "@xterm/xterm";

export type TerminalShortcuts = {
  copy: (selection: string) => void;
  openSearch: () => void;
};

export function isMacPlatform(): boolean {
  return (
    typeof navigator !== "undefined" &&
    /Mac|iPhone|iPad/.test(navigator.userAgent)
  );
}

export function terminalKeyHandler(
  terminal: Pick<Terminal, "hasSelection" | "getSelection" | "clearSelection">,
  shortcuts: TerminalShortcuts,
  isMac = isMacPlatform(),
): (event: KeyboardEvent) => boolean {
  return (event) => {
    if (event.type !== "keydown") return true;
    const primary = isMac ? event.metaKey : event.ctrlKey;
    const key = event.key.toLowerCase();
    if (key === "c" && primary && !event.altKey) {
      const copyChord = event.shiftKey || isMac;
      if (!copyChord && !terminal.hasSelection()) return true;
      if (terminal.hasSelection()) {
        shortcuts.copy(terminal.getSelection());
        terminal.clearSelection();
      }
      return false;
    }
    if (key === "insert" && !event.shiftKey && event.ctrlKey) {
      if (terminal.hasSelection()) shortcuts.copy(terminal.getSelection());
      return false;
    }
    if (key === "v" && primary && !event.altKey) return false;
    if (key === "insert" && event.shiftKey) return false;
    if (key === "f" && primary && (event.shiftKey || isMac)) {
      shortcuts.openSearch();
      return false;
    }
    return true;
  };
}
