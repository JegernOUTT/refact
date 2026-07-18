import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useTokens } from "../../../components/ui";
import { useConfig } from "../../../hooks";
import type { ExecStatus } from "../../../services/refact/exec";
import { useExecSession } from "./useExecSession";
import styles from "./TerminalPanel.module.css";

type TerminalSessionProps = {
  processId: string;
  apiKey?: string;
  onStatusChange: (processId: string, status: ExecStatus) => void;
};

export function TerminalSession({
  processId,
  apiKey,
  onStatusChange,
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
  const tokens = useTokens([
    "--rf-bg",
    "--rf-color-fg",
    "--rf-color-muted",
    "--rf-color-accent",
    "--rf-font-mono",
  ]);
  const background = tokens["--rf-bg"];
  const foreground = tokens["--rf-color-fg"];
  const muted = tokens["--rf-color-muted"];
  const accent = tokens["--rf-color-accent"];
  const fontFamily = tokens["--rf-font-mono"];
  const [runtime, setRuntime] = useState<{
    terminal: Terminal;
    fitAddon: FitAddon;
    container: HTMLElement;
  } | null>(null);
  const theme = useMemo(
    () => ({
      background,
      foreground,
      cursor: accent,
      selectionBackground: muted,
    }),
    [accent, background, foreground, muted],
  );

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const terminal = new Terminal({
      cursorBlink: true,
      fontFamily,
      theme,
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(container);
    setRuntime({ terminal, fitAddon, container });
    terminal.focus();

    return () => {
      terminal.dispose();
    };
  }, [fontFamily, theme]);

  const handleStatusChange = useCallback(
    (status: ExecStatus) => onStatusChange(processId, status),
    [onStatusChange, processId],
  );
  const { error, reconnecting } = useExecSession({
    processId,
    runtime,
    connection,
    apiKey,
    onStatusChange: handleStatusChange,
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
