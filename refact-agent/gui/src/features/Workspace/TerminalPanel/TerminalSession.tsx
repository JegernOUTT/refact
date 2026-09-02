import { ClipboardAddon } from "@xterm/addon-clipboard";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { Unicode11Addon } from "@xterm/addon-unicode11";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal, type ITheme } from "@xterm/xterm";
import { ChevronDown, ChevronUp, X } from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";

import { IconButton, useTokens } from "../../../components/ui";
import { useConfig, useCopyToClipboard, useOpenUrl } from "../../../hooks";
import type { ExecStatus } from "../../../services/refact/exec";
import { terminalKeyHandler } from "./terminalKeys";
import { useExecSession } from "./useExecSession";
import styles from "./TerminalPanel.module.css";

const THEME_TOKEN_NAMES = [
  "--rf-bg",
  "--rf-color-fg",
  "--rf-color-muted",
  "--rf-color-faint",
  "--rf-color-accent",
  "--rf-color-success",
  "--rf-color-warning",
  "--rf-color-danger",
  "--rf-chart-5",
  "--rf-chart-6",
  "--rf-font-mono",
  "--rf-text-2",
];

const TERMINAL_LINE_HEIGHT = 1.2;
const TERMINAL_SCROLLBACK_LINES = 5000;
const HIDE_CURSOR = "\u001b[?25l";
const SEARCH_OPTIONS = { caseSensitive: false, regex: false };

function usableToken(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  if (!trimmed || trimmed.includes("var(") || trimmed.includes("color-mix(")) {
    return undefined;
  }
  return trimmed;
}

function tokenPixels(value: string | undefined): number | undefined {
  const token = usableToken(value);
  if (!token?.endsWith("px")) return undefined;
  const pixels = Number.parseFloat(token);
  return Number.isFinite(pixels) && pixels > 0 ? pixels : undefined;
}

function tryLoadWebgl(terminal: Terminal): void {
  const webgl = new WebglAddon();
  try {
    terminal.loadAddon(webgl);
  } catch {
    webgl.dispose();
    return;
  }
  webgl.onContextLoss(() => webgl.dispose());
}

type TerminalSessionProps = {
  processId: string;
  chatId: string;
  apiKey?: string;
  readOnly?: boolean;
  focusRequest?: number;
  searchRequest?: number;
  onStatusChange: (
    processId: string,
    status: ExecStatus,
    exitCode?: number | null,
    endedAtMs?: number | null,
  ) => void;
  onResize?: (processId: string, rows: number, cols: number) => void;
};

export function TerminalSession({
  processId,
  chatId,
  apiKey,
  readOnly = false,
  focusRequest,
  searchRequest,
  onStatusChange,
  onResize,
}: TerminalSessionProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const config = useConfig();
  const openUrl = useOpenUrl();
  const copyToClipboard = useCopyToClipboard();
  const connection = useMemo(
    () => ({
      host: config.host,
      lspPort: config.lspPort,
      lspUrl: config.lspUrl,
      browserUrl: config.browserUrl,
      dev: config.dev,
      engineServed: config.engineServed,
    }),
    [
      config.browserUrl,
      config.dev,
      config.engineServed,
      config.host,
      config.lspPort,
      config.lspUrl,
    ],
  );
  const tokens = useTokens(THEME_TOKEN_NAMES);
  const background = usableToken(tokens["--rf-bg"]);
  const foreground = usableToken(tokens["--rf-color-fg"]);
  const muted = usableToken(tokens["--rf-color-muted"]);
  const faint = usableToken(tokens["--rf-color-faint"]);
  const accent = usableToken(tokens["--rf-color-accent"]);
  const success = usableToken(tokens["--rf-color-success"]);
  const warning = usableToken(tokens["--rf-color-warning"]);
  const danger = usableToken(tokens["--rf-color-danger"]);
  const cyan = usableToken(tokens["--rf-chart-5"]);
  const magenta = usableToken(tokens["--rf-chart-6"]);
  const fontFamily = usableToken(tokens["--rf-font-mono"]);
  const fontSize = tokenPixels(tokens["--rf-text-2"]);
  const theme = useMemo<ITheme>(
    () => ({
      background,
      foreground,
      cursor: accent,
      cursorAccent: background,
      selectionBackground: accent,
      selectionForeground: background,
      selectionInactiveBackground: muted,
      black: background,
      red: danger,
      green: success,
      yellow: warning,
      blue: accent,
      magenta,
      cyan,
      white: foreground,
      brightBlack: faint ?? muted,
      brightRed: danger,
      brightGreen: success,
      brightYellow: warning,
      brightBlue: accent,
      brightMagenta: magenta,
      brightCyan: cyan,
      brightWhite: foreground,
    }),
    [
      accent,
      background,
      cyan,
      danger,
      faint,
      foreground,
      magenta,
      muted,
      success,
      warning,
    ],
  );
  const themeRef = useRef(theme);
  themeRef.current = theme;
  const fontFamilyRef = useRef(fontFamily);
  fontFamilyRef.current = fontFamily;
  const fontSizeRef = useRef(fontSize);
  fontSizeRef.current = fontSize;
  const openUrlRef = useRef(openUrl);
  openUrlRef.current = openUrl;
  const copyRef = useRef(copyToClipboard);
  copyRef.current = copyToClipboard;
  const [runtime, setRuntime] = useState<{
    terminal: Terminal;
    fitAddon: FitAddon;
    searchAddon: SearchAddon;
    container: HTMLElement;
  } | null>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const terminal = new Terminal({
      allowProposedApi: true,
      convertEol: readOnly,
      cursorBlink: !readOnly,
      cursorInactiveStyle: readOnly ? "none" : "outline",
      disableStdin: readOnly,
      fontFamily: fontFamilyRef.current,
      fontSize: fontSizeRef.current,
      lineHeight: TERMINAL_LINE_HEIGHT,
      macOptionIsMeta: true,
      scrollback: TERMINAL_SCROLLBACK_LINES,
      theme: themeRef.current,
    });
    const fitAddon = new FitAddon();
    const searchAddon = new SearchAddon();
    terminal.loadAddon(fitAddon);
    terminal.loadAddon(searchAddon);
    terminal.loadAddon(new Unicode11Addon());
    terminal.loadAddon(new ClipboardAddon());
    terminal.loadAddon(
      new WebLinksAddon((event, uri) => {
        event.preventDefault();
        openUrlRef.current(uri);
      }),
    );
    terminal.unicode.activeVersion = "11";
    terminal.attachCustomKeyEventHandler(
      terminalKeyHandler(terminal, {
        copy: (selection) => {
          copyRef.current(selection);
          terminal.focus();
        },
        openSearch: () => setSearchOpen(true),
      }),
    );
    terminal.open(container);
    tryLoadWebgl(terminal);
    if (readOnly) terminal.write(HIDE_CURSOR);
    setRuntime({ terminal, fitAddon, searchAddon, container });
    if (!readOnly) terminal.focus();

    return () => {
      setRuntime(null);
      terminal.dispose();
    };
  }, [readOnly]);

  useEffect(() => {
    if (!runtime) return;
    runtime.terminal.options.theme = theme;
    if (fontFamily) runtime.terminal.options.fontFamily = fontFamily;
    if (fontSize) runtime.terminal.options.fontSize = fontSize;
  }, [fontFamily, fontSize, runtime, theme]);

  useEffect(() => {
    if (!runtime || readOnly || focusRequest === undefined) return;
    runtime.terminal.focus();
  }, [focusRequest, readOnly, runtime]);

  useEffect(() => {
    if (searchRequest === undefined || searchRequest === 0) return;
    setSearchOpen(true);
  }, [searchRequest]);

  useEffect(() => {
    if (!searchOpen) return;
    searchInputRef.current?.focus();
    searchInputRef.current?.select();
  }, [searchOpen]);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    runtime?.searchAddon.clearDecorations();
    runtime?.terminal.clearSelection();
    if (!readOnly) runtime?.terminal.focus();
  }, [readOnly, runtime]);

  const handleSearchChange = useCallback(
    (event: ChangeEvent<HTMLInputElement>) => {
      const query = event.target.value;
      setSearchQuery(query);
      runtime?.searchAddon.findNext(query, {
        ...SEARCH_OPTIONS,
        incremental: true,
      });
    },
    [runtime],
  );

  const findNext = useCallback(() => {
    runtime?.searchAddon.findNext(searchQuery, SEARCH_OPTIONS);
  }, [runtime, searchQuery]);

  const findPrevious = useCallback(() => {
    runtime?.searchAddon.findPrevious(searchQuery, SEARCH_OPTIONS);
  }, [runtime, searchQuery]);

  const handleSearchKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLInputElement>) => {
      if (event.key === "Enter") {
        event.preventDefault();
        if (event.shiftKey) findPrevious();
        else findNext();
      } else if (event.key === "Escape") {
        event.preventDefault();
        closeSearch();
      }
    },
    [closeSearch, findNext, findPrevious],
  );

  const handleStatusChange = useCallback(
    (status: ExecStatus, exitCode?: number | null, endedAtMs?: number | null) =>
      onStatusChange(processId, status, exitCode, endedAtMs),
    [onStatusChange, processId],
  );
  const handleResize = useCallback(
    (rows: number, cols: number) => onResize?.(processId, rows, cols),
    [onResize, processId],
  );
  const { error, reconnecting } = useExecSession({
    processId,
    chatId,
    runtime,
    connection,
    apiKey,
    onStatusChange: handleStatusChange,
    onResize: handleResize,
    interactive: !readOnly,
  });

  return (
    <div className={styles.session} data-terminal-process-id={processId}>
      <div ref={containerRef} className={styles.terminal} />
      {searchOpen ? (
        <div className={styles.search} role="search">
          <input
            ref={searchInputRef}
            className={styles.searchInput}
            aria-label="Find in terminal"
            placeholder="Find"
            spellCheck={false}
            value={searchQuery}
            onChange={handleSearchChange}
            onKeyDown={handleSearchKeyDown}
          />
          <IconButton
            icon={ChevronUp}
            aria-label="Previous match"
            size="sm"
            variant="plain"
            onClick={findPrevious}
          />
          <IconButton
            icon={ChevronDown}
            aria-label="Next match"
            size="sm"
            variant="plain"
            onClick={findNext}
          />
          <IconButton
            icon={X}
            aria-label="Close find"
            size="sm"
            variant="plain"
            onClick={closeSearch}
          />
        </div>
      ) : null}
      {reconnecting ? (
        <div className={styles.connectionNotice}>Reconnecting terminal…</div>
      ) : null}
      {error ? (
        <div className={styles.errorNotice} role="alert">
          {error}
        </div>
      ) : null}
    </div>
  );
}
