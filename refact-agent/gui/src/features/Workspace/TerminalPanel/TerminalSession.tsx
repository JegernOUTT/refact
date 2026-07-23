import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITheme } from "@xterm/xterm";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useTokens } from "../../../components/ui";
import { useConfig } from "../../../hooks";
import type { ExecStatus } from "../../../services/refact/exec";
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
];

function usableToken(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  if (!trimmed || trimmed.includes("var(") || trimmed.includes("color-mix(")) {
    return undefined;
  }
  return trimmed;
}

type TerminalSessionProps = {
  processId: string;
  apiKey?: string;
  onStatusChange: (processId: string, status: ExecStatus) => void;
  onResize?: (processId: string, rows: number, cols: number) => void;
};

export function TerminalSession({
  processId,
  apiKey,
  onStatusChange,
  onResize,
}: TerminalSessionProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const config = useConfig();
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
  const theme = useMemo<ITheme>(
    () => ({
      background,
      foreground,
      cursor: accent,
      cursorAccent: background,
      selectionBackground: muted,
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
  const [runtime, setRuntime] = useState<{
    terminal: Terminal;
    fitAddon: FitAddon;
    container: HTMLElement;
  } | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const terminal = new Terminal({
      cursorBlink: true,
      fontFamily: fontFamilyRef.current,
      theme: themeRef.current,
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(container);
    setRuntime({ terminal, fitAddon, container });
    terminal.focus();

    return () => {
      setRuntime(null);
      terminal.dispose();
    };
  }, []);

  useEffect(() => {
    if (!runtime) return;
    runtime.terminal.options.theme = theme;
    if (fontFamily) runtime.terminal.options.fontFamily = fontFamily;
  }, [fontFamily, runtime, theme]);

  const handleStatusChange = useCallback(
    (status: ExecStatus) => onStatusChange(processId, status),
    [onStatusChange, processId],
  );
  const handleResize = useCallback(
    (rows: number, cols: number) => onResize?.(processId, rows, cols),
    [onResize, processId],
  );
  const { error, reconnecting } = useExecSession({
    processId,
    runtime,
    connection,
    apiKey,
    onStatusChange: handleStatusChange,
    onResize: handleResize,
  });

  return (
    <div className={styles.session} data-terminal-process-id={processId}>
      <div ref={containerRef} className={styles.terminal} />
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
