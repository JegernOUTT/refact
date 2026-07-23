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

type TerminalRuntime = {
  terminal: Terminal;
  fitAddon: FitAddon;
  container: HTMLElement;
};

type UseExecSessionOptions = {
  processId: string;
  runtime: TerminalRuntime | null;
  connection: EngineApiConnection;
  apiKey?: string;
  onStatusChange: (status: ExecStatus) => void;
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

export function useExecSession({
  processId,
  runtime,
  connection,
  apiKey,
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

    const updateStatus = (status: ExecStatus) => {
      statusRef.current = status;
      onStatusChange(status);
    };

    const writeChunks = (chunks: ExecOutputChunk[]) => {
      for (const chunk of chunks) {
        if (chunk.seq < nextSequenceRef.current) continue;
        terminal.write(chunk.text);
        nextSequenceRef.current = chunk.offset ?? chunk.seq + 1;
      }
    };

    const writeExitNotice = (status: ExecStatus) => {
      if (exitNoticeWritten) return;
      exitNoticeWritten = true;
      terminal.write(`\r\n[process exited: ${status}]\r\n`);
    };

    const flushInput = async () => {
      inputTimer = null;
      const chars = inputBuffer;
      inputBuffer = "";
      if (!chars || stopped) return;
      try {
        await writeProcessStdin(processId, chars, connection, apiKey);
      } catch (cause) {
        reportError(cause);
      }
    };

    const dataDisposable = terminal.onData((chars) => {
      inputBuffer += chars;
      if (inputTimer === null) {
        inputTimer = setTimeout(() => void flushInput(), INPUT_DEBOUNCE_MS);
      }
    });

    const syncSize = async () => {
      if (stopped || isTerminalStatus(statusRef.current)) return;
      try {
        fitAddon.fit();
      } catch {
        return;
      }
      const { rows, cols } = terminal;
      if (rows <= 0 || cols <= 0) return;
      onResize?.(rows, cols);
      if (syncedSize && syncedSize.rows === rows && syncedSize.cols === cols) {
        return;
      }
      try {
        await resizeExec(processId, rows, cols, connection, apiKey);
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
        execSubscribeUrl(processId, connection, nextSequenceRef.current),
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
        updateStatus(snapshot.status);
      });
      eventSource.addEventListener("output", (event) => {
        const chunk = parseEvent<ExecOutputChunk>(event);
        if (chunk) writeChunks([chunk]);
      });
      eventSource.addEventListener("exit", (event) => {
        const exit = parseEvent<ExecExitEvent>(event);
        if (!exit) return;
        updateStatus(exit.status);
        writeExitNotice(exit.status);
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
          apiKey,
          true,
        );
        if (stopped) return;
        writeChunks(read.chunks);
        nextSequenceRef.current = Math.max(
          nextSequenceRef.current,
          read.next_seq,
        );
        updateStatus(read.status);
        if (isTerminalStatus(read.status)) {
          writeExitNotice(read.status);
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
      dataDisposable.dispose();
      resizeObserver.disconnect();
      eventSource?.close();
      if (inputTimer !== null) clearTimeout(inputTimer);
      if (resizeTimer !== null) clearTimeout(resizeTimer);
      if (reconnectTimer !== null) clearTimeout(reconnectTimer);
    };
  }, [apiKey, connection, onResize, onStatusChange, processId, runtime]);

  return { error, reconnecting };
}
