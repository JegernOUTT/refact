import "@xterm/xterm/css/xterm.css";

import { Plus, SquareTerminal, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  Button,
  EmptyState,
  IconButton,
  StatusDot,
} from "../../../components/ui";
import { useAppDispatch, useAppSelector, useConfig } from "../../../hooks";
import {
  ExecHttpError,
  killExec,
  listExec,
  spawnExec,
  type ExecStatus,
} from "../../../services/refact/exec";
import { TerminalSession } from "./TerminalSession";
import {
  activeSessionChanged,
  selectActiveTerminalProcessId,
  selectTerminalSessions,
  sessionAdded,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
} from "./terminalSlice";
import styles from "./TerminalPanel.module.css";

const DEFAULT_PTY_ROWS = 24;
const DEFAULT_PTY_COLS = 80;

function shortProcessId(processId: string): string {
  return processId.slice(0, 8);
}

function terminalTitle(
  processId: string,
  commandPreview: string | undefined,
): string {
  const label = commandPreview?.trim();
  return `${label && label.length > 0 ? label : "shell"} · ${shortProcessId(
    processId,
  )}`;
}

function statusDot(status: ExecStatus): "running" | "error" | "idle" {
  if (status === "running" || status === "starting") return "running";
  if (status === "failed" || status === "timed_out") return "error";
  return "idle";
}

export function TerminalPanel() {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const sessions = useAppSelector(selectTerminalSessions);
  const activeProcessId = useAppSelector(selectActiveTerminalProcessId);
  const [loading, setLoading] = useState(true);
  const [spawning, setSpawning] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const lastFittedRef = useRef<{ rows: number; cols: number } | null>(null);
  const apiKey = config.apiKey ?? undefined;
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

  useEffect(() => {
    let cancelled = false;
    void listExec(connection, apiKey)
      .then((response) => {
        if (cancelled) return;
        dispatch(
          sessionsReattached(
            response.processes
              .filter((process) => process.tty && process.status === "running")
              .map((process) => ({
                process_id: process.process_id,
                title: terminalTitle(
                  process.process_id,
                  process.command_preview,
                ),
                status: process.status,
              })),
          ),
        );
      })
      .catch((cause: unknown) => {
        if (!cancelled) {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [apiKey, connection, dispatch]);

  const handleNewSession = useCallback(async () => {
    setSpawning(true);
    setError(null);
    try {
      const fitted = lastFittedRef.current;
      const result = await spawnExec(
        {
          pty: true,
          rows: fitted?.rows ?? DEFAULT_PTY_ROWS,
          cols: fitted?.cols ?? DEFAULT_PTY_COLS,
        },
        connection,
        apiKey,
      );
      dispatch(
        sessionAdded({
          process_id: result.process_id,
          title: terminalTitle(result.process_id, result.command_preview),
          status: result.status,
        }),
      );
    } catch (cause) {
      if (cause instanceof ExecHttpError && cause.status === 403) {
        setDisabled(true);
      } else {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    } finally {
      setSpawning(false);
    }
  }, [apiKey, connection, dispatch]);

  const handleCloseSession = useCallback(
    async (processId: string, status: ExecStatus) => {
      const running = status === "running" || status === "starting";
      if (
        running &&
        !window.confirm("This terminal is still running. Stop and close it?")
      ) {
        return;
      }
      setError(null);
      try {
        if (running) await killExec(processId, connection, apiKey);
        dispatch(sessionRemoved(processId));
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    },
    [apiKey, connection, dispatch],
  );

  const handleStatusChange = useCallback(
    (processId: string, status: ExecStatus) => {
      dispatch(sessionStatusChanged({ processId, status }));
    },
    [dispatch],
  );

  const handleSessionResize = useCallback(
    (_processId: string, rows: number, cols: number) => {
      lastFittedRef.current = { rows, cols };
    },
    [],
  );

  if (disabled) {
    return (
      <div className={styles.fullState}>
        <EmptyState
          icon={SquareTerminal}
          title="Browser terminal disabled"
          description="Terminal access is disabled by the daemon or REFACT_DISABLE_EXEC_HTTP policy. Enable exec HTTP access and try again."
          variant="full"
          action={<Button onClick={() => setDisabled(false)}>Try again</Button>}
        />
      </div>
    );
  }

  return (
    <section className={styles.panel} aria-label="Terminal panel">
      <header className={styles.header}>
        <div
          className={styles.tabs}
          role="tablist"
          aria-label="Terminal sessions"
        >
          {sessions.map((session) => {
            const active = session.process_id === activeProcessId;
            return (
              <div
                key={session.process_id}
                className={active ? styles.tabActive : styles.tab}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={active}
                  className={styles.tabSelect}
                  onClick={() =>
                    dispatch(activeSessionChanged(session.process_id))
                  }
                >
                  <StatusDot status={statusDot(session.status)} />
                  <span className={styles.tabTitle}>{session.title}</span>
                </button>
                <IconButton
                  icon={X}
                  aria-label={`Close ${session.title}`}
                  size="sm"
                  variant="plain"
                  className={styles.closeButton}
                  onClick={() =>
                    void handleCloseSession(session.process_id, session.status)
                  }
                />
              </div>
            );
          })}
        </div>
        <IconButton
          icon={Plus}
          aria-label="New terminal"
          size="sm"
          variant="plain"
          loading={spawning}
          onClick={() => void handleNewSession()}
        />
      </header>

      <div className={styles.body}>
        {sessions.map((session) => (
          <div
            key={session.process_id}
            className={
              session.process_id === activeProcessId
                ? styles.sessionActive
                : styles.sessionHidden
            }
            aria-hidden={session.process_id !== activeProcessId}
          >
            <TerminalSession
              processId={session.process_id}
              apiKey={apiKey}
              onStatusChange={handleStatusChange}
              onResize={handleSessionResize}
            />
          </div>
        ))}
        {!loading && sessions.length === 0 ? (
          <EmptyState
            icon={SquareTerminal}
            title="No terminal sessions"
            description="Start an interactive shell in the active workspace."
            variant="full"
            action={
              <Button
                leftIcon={Plus}
                loading={spawning}
                onClick={() => void handleNewSession()}
              >
                New terminal
              </Button>
            }
          />
        ) : null}
        {loading ? (
          <div className={styles.loading}>Finding terminal sessions…</div>
        ) : null}
      </div>

      {error ? (
        <div className={styles.panelError} role="alert">
          {error}
        </div>
      ) : null}
    </section>
  );
}
