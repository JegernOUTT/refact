import type { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import { useEffect, useRef, useState } from "react";

import type { EngineApiConnection } from "../../../services/refact/chatCommands";
import {
  ExecHttpError,
  execSubscribeUrl,
  readExec,
  resizeExec,
  writeProcessStdin,
  type ExecExitEvent,
  type ExecOutputChunk,
  type ExecSnapshotEvent,
  type ExecStatus,
} from "../../../services/refact/exec";

const INPUT_DEBOUNCE_MS = 16;
const RESIZE_DEBOUNCE_MS = 150;
const INITIAL_RECONNECT_DELAY_MS = 250;
const MAX_RECONNECT_DELAY_MS = 5_000;
const DIM = "\u001b[2m";
const DIM_OFF = "\u001b[22m";

type TerminalRuntime = {
  terminal: Terminal;
  fitAddon: FitAddon;
  container: HTMLElement;
};

type UseExecSessionOptions = {
  processId: string;
  chatId: string;
  runtime: TerminalRuntime | null;
  connection: EngineApiConnection;
  apiKey?: string;
  interactive?: boolean;
  onStatusChange: (
    status: ExecStatus,
    exitCode?: number | null,
    endedAtMs?: number | null,
  ) => void;
  onResize?: (rows: number, cols: number) => void;
};

function parseEvent<T>(event: Event): T | null {
  if (!(event instanceof MessageEvent) || typeof event.data !== "string") {
    return null;
  }
  try {
    return JSON.parse(event.data) as T;
  } catch {
    return null;
  }
}

function isTerminalStatus(status: ExecStatus): boolean {
  return !["starting", "running"].includes(status);
}

function commandHeader(command: string): string {
  const text = Array.from(command.replace(/\r\n?|\n/g, "\n"), (char) => {
    const code = char.charCodeAt(0);
    // Metadata is display-only: escape terminal controls, including ESC and C1.
    return (code < 32 && char !== "\n") || (code >= 127 && code <= 159)
      ? `\\x${code.toString(16).padStart(2, "0")}`
      : char;
  }).join("");
  return `${DIM}$ ${text.replace(/\n/g, "\r\n")}${DIM_OFF}\r\n`;
}

export function exitNoticeText(
  status: ExecStatus,
  exitCode?: number | null,
): string {
  switch (status) {
    case "killed":
      return "process killed";
    case "timed_out":
      return "process timed out";
    case "failed":
      return "process failed";
    default:
      return exitCode === undefined || exitCode === null
        ? "process exited"
        : `process exited with code ${exitCode}`;
  }
}

export function useExecSession({
  processId,
  chatId,
  runtime,
  connection,
  apiKey,
  interactive = true,
  onStatusChange,
  onResize,
}: UseExecSessionOptions) {
  const [error, setError] = useState<string | null>(null);
  const [reconnecting, setReconnecting] = useState(false);
  const statusRef = useRef<ExecStatus>("running");
  const nextSequenceRef = useRef(0);
  const historyRef = useRef<{ terminal: Terminal; started: boolean } | null>(
    null,
  );

  useEffect(() => {
    if (!runtime) return;

    setError(null);
    setReconnecting(false);
    const { container, fitAddon, terminal } = runtime;
    if (historyRef.current?.terminal !== terminal) {
      historyRef.current = { terminal, started: false };
    }
    const history = historyRef.current;
    let stopped = false;
    const isStopped = () => stopped;
    let eventSource: EventSource | null = null;
    let inputTimer: ReturnType<typeof setTimeout> | null = null;
    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
    let inputBuffer = "";
    let exitNoticeWritten = false;
    let syncedSize: { rows: number; cols: number } | null = null;

    const clearTimers = () => {
      if (inputTimer !== null) clearTimeout(inputTimer);
      if (resizeTimer !== null) clearTimeout(resizeTimer);
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
      inputTimer = resizeTimer = reconnectTimer = null;
    };

    const reportError = (cause: unknown) => {
      if (isStopped()) return;
      setError(cause instanceof Error ? cause.message : String(cause));
      if (
        cause instanceof ExecHttpError &&
        [401, 403, 404].includes(cause.status)
      ) {
        stopped = true;
        clearTimers();
        inputBuffer = "";
        eventSource?.close();
        eventSource = null;
        setReconnecting(false);
      }
    };

    const updateStatus = (
      status: ExecStatus,
      exitCode?: number | null,
      endedAtMs?: number | null,
    ) => {
      statusRef.current = status;
      onStatusChange(
        status,
        exitCode,
        isTerminalStatus(status) ? endedAtMs ?? Date.now() : undefined,
      );
    };

    const writeCommand = (command?: string) => {
      if (history.started || typeof command !== "string") return;
      history.started = true;
      terminal.write(commandHeader(command));
    };

    const writeChunks = (chunks: ExecOutputChunk[]) => {
      for (const chunk of chunks) {
        if (chunk.seq < nextSequenceRef.current) continue;
        history.started = true;
        terminal.write(chunk.text);
        nextSequenceRef.current = chunk.offset ?? chunk.seq + 1;
      }
    };

    const writeExitNotice = (status: ExecStatus, exitCode?: number | null) => {
      if (exitNoticeWritten) return;
      exitNoticeWritten = true;
      history.started = true;
      terminal.write(
        `\r\n${DIM}[${exitNoticeText(status, exitCode)}]${DIM_OFF}\r\n`,
      );
    };

    const flushInput = async () => {
      inputTimer = null;
      const chars = inputBuffer;
      inputBuffer = "";
      if (!chars || isStopped()) return;
      try {
        await writeProcessStdin(processId, chars, connection, chatId, apiKey);
      } catch (cause) {
        reportError(cause);
      }
    };

    const dataDisposable = interactive
      ? terminal.onData((chars) => {
          if (isStopped()) return;
          inputBuffer += chars;
          if (inputTimer === null) {
            inputTimer = setTimeout(() => void flushInput(), INPUT_DEBOUNCE_MS);
          }
        })
      : null;

    const syncSize = async () => {
      if (isStopped()) return;
      try {
        fitAddon.fit();
      } catch {
        return;
      }
      const { rows, cols } = terminal;
      if (rows <= 0 || cols <= 0) return;
      onResize?.(rows, cols);
      if (!interactive || isTerminalStatus(statusRef.current)) return;
      if (syncedSize && syncedSize.rows === rows && syncedSize.cols === cols) {
        return;
      }
      try {
        await resizeExec(processId, rows, cols, connection, chatId, apiKey);
        if (isStopped()) return;
        syncedSize = { rows, cols };
      } catch (cause) {
        reportError(cause);
      }
    };

    const scheduleResize = () => {
      if (isStopped()) return;
      if (resizeTimer !== null) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => {
        resizeTimer = null;
        void syncSize();
      }, RESIZE_DEBOUNCE_MS);
    };
    const resizeObserver = new ResizeObserver(scheduleResize);
    resizeObserver.observe(container);
    const fonts = (document as Partial<Document>).fonts;
    void fonts?.ready.then(() => scheduleResize());

    const scheduleReconnect = () => {
      eventSource?.close();
      eventSource = null;
      if (
        isStopped() ||
        reconnectTimer !== null ||
        isTerminalStatus(statusRef.current)
      ) {
        return;
      }
      setReconnecting(true);
      const delay = reconnectDelay;
      reconnectDelay = Math.min(reconnectDelay * 2, MAX_RECONNECT_DELAY_MS);
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
        void backfillAndConnect();
      }, delay);
    };

    const connect = () => {
      if (isStopped() || isTerminalStatus(statusRef.current)) return;
      const source = new EventSource(
        execSubscribeUrl(
          processId,
          connection,
          chatId,
          nextSequenceRef.current,
        ),
      );
      eventSource = source;
      const isCurrent = () => !isStopped() && eventSource === source;
      source.onopen = () => {
        if (!isCurrent()) return;
        reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
        setReconnecting(false);
        setError(null);
      };
      source.addEventListener("snapshot", (event) => {
        if (!isCurrent()) return;
        const snapshot = parseEvent<ExecSnapshotEvent>(event);
        if (!snapshot) return;
        writeCommand(snapshot.command);
        writeChunks(snapshot.chunks);
        nextSequenceRef.current = Math.max(
          nextSequenceRef.current,
          snapshot.next_seq,
        );
        updateStatus(snapshot.status, snapshot.exit_code);
      });
      source.addEventListener("output", (event) => {
        if (!isCurrent()) return;
        const chunk = parseEvent<ExecOutputChunk>(event);
        if (chunk) writeChunks([chunk]);
      });
      source.addEventListener("exit", (event) => {
        if (!isCurrent()) return;
        const exit = parseEvent<ExecExitEvent>(event);
        if (!exit) return;
        updateStatus(exit.status, exit.exit_code, exit.ended_at_ms);
        writeExitNotice(exit.status, exit.exit_code);
        eventSource?.close();
        eventSource = null;
        setReconnecting(false);
      });
      source.onerror = () => {
        if (isCurrent()) scheduleReconnect();
      };
    };

    async function backfillAndConnect() {
      if (isStopped()) return;
      await syncSize();
      if (isStopped()) return;
      try {
        const read = await readExec(
          processId,
          nextSequenceRef.current,
          connection,
          chatId,
          apiKey,
          interactive,
        );
        if (isStopped()) return;
        writeCommand(read.command);
        writeChunks(read.chunks);
        nextSequenceRef.current = Math.max(
          nextSequenceRef.current,
          read.next_seq,
        );
        updateStatus(read.status, read.exit_code, read.ended_at_ms);
        if (isTerminalStatus(read.status)) {
          writeExitNotice(read.status, read.exit_code);
          setReconnecting(false);
          return;
        }
        connect();
      } catch (cause) {
        if (isStopped()) return;
        reportError(cause);
        scheduleReconnect();
      }
    }

    void backfillAndConnect();

    return () => {
      stopped = true;
      dataDisposable?.dispose();
      resizeObserver.disconnect();
      eventSource?.close();
      clearTimers();
    };
  }, [
    apiKey,
    chatId,
    connection,
    interactive,
    onResize,
    onStatusChange,
    processId,
    runtime,
  ]);

  return { error, reconnecting };
}
