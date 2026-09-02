import type { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import { useEffect, useRef, useState } from "react";

import type { EngineApiConnection } from "../../../services/refact/chatCommands";
import {
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

  useEffect(() => {
    if (!runtime) return;

    const { container, fitAddon, terminal } = runtime;
    let stopped = false;
    let eventSource: EventSource | null = null;
    let inputTimer: ReturnType<typeof setTimeout> | null = null;
    let resizeTimer: ReturnType<typeof setTimeout> | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
    let inputBuffer = "";
    let exitNoticeWritten = false;
    let syncedSize: { rows: number; cols: number } | null = null;

    const reportError = (cause: unknown) => {
      if (stopped) return;
      setError(cause instanceof Error ? cause.message : String(cause));
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

    const writeChunks = (chunks: ExecOutputChunk[]) => {
      for (const chunk of chunks) {
        if (chunk.seq < nextSequenceRef.current) continue;
        terminal.write(chunk.text);
        nextSequenceRef.current = chunk.offset ?? chunk.seq + 1;
      }
    };

    const writeExitNotice = (status: ExecStatus, exitCode?: number | null) => {
      if (exitNoticeWritten) return;
      exitNoticeWritten = true;
      terminal.write(
        `\r\n${DIM}[${exitNoticeText(status, exitCode)}]${DIM_OFF}\r\n`,
      );
    };

    const flushInput = async () => {
      inputTimer = null;
      const chars = inputBuffer;
      inputBuffer = "";
      if (!chars || stopped) return;
      try {
        await writeProcessStdin(processId, chars, connection, chatId, apiKey);
      } catch (cause) {
        reportError(cause);
      }
    };

    const dataDisposable = interactive
      ? terminal.onData((chars) => {
          inputBuffer += chars;
          if (inputTimer === null) {
            inputTimer = setTimeout(() => void flushInput(), INPUT_DEBOUNCE_MS);
          }
        })
      : null;

    const syncSize = async () => {
      if (stopped) return;
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
        syncedSize = { rows, cols };
      } catch (cause) {
        reportError(cause);
      }
    };

    const scheduleResize = () => {
      if (stopped) return;
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
        stopped ||
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
      if (stopped || isTerminalStatus(statusRef.current)) return;
      eventSource = new EventSource(
        execSubscribeUrl(
          processId,
          connection,
          chatId,
          nextSequenceRef.current,
        ),
      );
      eventSource.onopen = () => {
        reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
        setReconnecting(false);
        setError(null);
      };
      eventSource.addEventListener("snapshot", (event) => {
        const snapshot = parseEvent<ExecSnapshotEvent>(event);
        if (!snapshot) return;
        writeChunks(snapshot.chunks);
        nextSequenceRef.current = Math.max(
          nextSequenceRef.current,
          snapshot.next_seq,
        );
        updateStatus(snapshot.status, snapshot.exit_code);
      });
      eventSource.addEventListener("output", (event) => {
        const chunk = parseEvent<ExecOutputChunk>(event);
        if (chunk) writeChunks([chunk]);
      });
      eventSource.addEventListener("exit", (event) => {
        const exit = parseEvent<ExecExitEvent>(event);
        if (!exit) return;
        updateStatus(exit.status, exit.exit_code, exit.ended_at_ms);
        writeExitNotice(exit.status, exit.exit_code);
        eventSource?.close();
        eventSource = null;
        setReconnecting(false);
      });
      eventSource.onerror = scheduleReconnect;
    };

    async function backfillAndConnect() {
      await syncSize();
      try {
        const read = await readExec(
          processId,
          nextSequenceRef.current,
          connection,
          chatId,
          apiKey,
          interactive,
        );
        if (stopped) return;
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
        if (stopped) return;
        setError(cause instanceof Error ? cause.message : String(cause));
        scheduleReconnect();
      }
    }

    void backfillAndConnect();

    return () => {
      stopped = true;
      dataDisposable?.dispose();
      resizeObserver.disconnect();
      eventSource?.close();
      if (inputTimer !== null) clearTimeout(inputTimer);
      if (resizeTimer !== null) clearTimeout(resizeTimer);
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
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
