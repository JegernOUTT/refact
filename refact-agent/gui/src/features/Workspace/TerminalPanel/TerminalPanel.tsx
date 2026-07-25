import "@xterm/xterm/css/xterm.css";

import { ChevronDown, ChevronUp, Plus, SquareTerminal, X } from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";

import {
  Button,
  EmptyState,
  Icon,
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
import { selectChatWorkspaceRoot } from "../workspaceSlice";
import {
  activeSessionChanged,
  selectActiveTerminalProcessId,
  selectTerminalSessions,
  selectTerminalWorkbenchOpen,
  sessionAdded,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
  setTerminalWorkbenchOpen,
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

export function TerminalPanel({ chatId }: { chatId: string }) {
  return <ChatTerminalPanel key={chatId} chatId={chatId} />;
}

function ChatTerminalPanel({ chatId }: { chatId: string }) {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const workspaceRoot = useAppSelector((state) =>
    selectChatWorkspaceRoot(state, chatId),
  );
  const sessions = useAppSelector((state) =>
    selectTerminalSessions(state, chatId),
  );
  const activeProcessId = useAppSelector((state) =>
    selectActiveTerminalProcessId(state, chatId),
  );
  const workbenchOpen = useAppSelector((state) =>
    selectTerminalWorkbenchOpen(state, chatId),
  );
  const [loading, setLoading] = useState(true);
  const [spawning, setSpawning] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [listAttempt, setListAttempt] = useState(0);
  const lastFittedRef = useRef<{ rows: number; cols: number } | null>(null);
  const tabListId = useId();
  const tabRefs = useRef(new Map<string, HTMLButtonElement>());
  const [focusedProcessId, setFocusedProcessId] = useState<string | null>(null);
  const [terminalFocusRequest, setTerminalFocusRequest] = useState(0);
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
    setLoading(true);
    setDisabled(false);
    setError(null);
    let cancelled = false;
    void listExec(connection, apiKey, chatId)
      .then((response) => {
        if (cancelled) return;
        setDisabled(false);
        setError(null);
        dispatch(
          sessionsReattached({
            chatId,
            sessions: response.processes
              .filter((process) => process.tty)
              .map((process) => ({
                process_id: process.process_id,
                title: terminalTitle(
                  process.process_id,
                  process.command_preview,
                ),
                status: process.status,
              })),
          }),
        );
      })
      .catch((cause: unknown) => {
        if (cancelled) return;
        if (cause instanceof ExecHttpError && cause.status === 403) {
          setDisabled(true);
          setError(null);
        } else {
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [apiKey, chatId, connection, dispatch, listAttempt]);

  const handleNewSession = useCallback(async () => {
    dispatch(setTerminalWorkbenchOpen({ chatId, open: true }));
    setSpawning(true);
    setError(null);
    try {
      const fitted = lastFittedRef.current;
      const result = await spawnExec(
        {
          chat_id: chatId,
          ...(workspaceRoot ? { cwd: workspaceRoot } : {}),
          pty: true,
          rows: fitted?.rows ?? DEFAULT_PTY_ROWS,
          cols: fitted?.cols ?? DEFAULT_PTY_COLS,
        },
        connection,
        apiKey,
      );
      dispatch(
        sessionAdded({
          chatId,
          session: {
            process_id: result.process_id,
            title: terminalTitle(result.process_id, result.command_preview),
            status: result.status,
          },
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
  }, [apiKey, chatId, connection, dispatch, workspaceRoot]);

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
        if (running) await killExec(processId, connection, chatId, apiKey);
        dispatch(sessionRemoved({ chatId, processId }));
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    },
    [apiKey, chatId, connection, dispatch],
  );

  const handleStatusChange = useCallback(
    (processId: string, status: ExecStatus) => {
      dispatch(sessionStatusChanged({ chatId, processId, status }));
    },
    [chatId, dispatch],
  );

  const handleSessionResize = useCallback(
    (_processId: string, rows: number, cols: number) => {
      lastFittedRef.current = { rows, cols };
    },
    [],
  );

  useEffect(() => {
    if (
      focusedProcessId &&
      sessions.some((session) => session.process_id === focusedProcessId)
    ) {
      return;
    }
    setFocusedProcessId(activeProcessId);
  }, [activeProcessId, focusedProcessId, sessions]);

  const activateSession = useCallback(
    (processId: string) => {
      setFocusedProcessId(processId);
      setTerminalFocusRequest((request) => request + 1);
      dispatch(activeSessionChanged({ chatId, processId }));
    },
    [chatId, dispatch],
  );

  const handleTabKeyDown = useCallback(
    (event: KeyboardEvent<HTMLButtonElement>, processId: string) => {
      const currentIndex = sessions.findIndex(
        (session) => session.process_id === processId,
      );
      if (currentIndex < 0 || sessions.length === 0) return;
      let nextIndex: number;
      switch (event.key) {
        case "ArrowLeft":
          nextIndex = (currentIndex - 1 + sessions.length) % sessions.length;
          break;
        case "ArrowRight":
          nextIndex = (currentIndex + 1) % sessions.length;
          break;
        case "Home":
          nextIndex = 0;
          break;
        case "End":
          nextIndex = sessions.length - 1;
          break;
        default:
          return;
      }
      event.preventDefault();
      const nextProcessId = sessions[nextIndex].process_id;
      setFocusedProcessId(nextProcessId);
      tabRefs.current.get(nextProcessId)?.focus();
    },
    [sessions],
  );

  return (
    <section
      className={styles.panel}
      aria-label={`Terminal workbench for ${chatId}`}
      data-open={workbenchOpen}
    >
      <header className={styles.header}>
        <div className={styles.title}>
          <Icon icon={SquareTerminal} size="sm" tone="muted" />
          <span>Terminal</span>
        </div>
        <div
          className={styles.tabs}
          role="tablist"
          aria-label="Terminal sessions"
        >
          {sessions.map((session, index) => {
            const active = session.process_id === activeProcessId;
            const tabId = `${tabListId}-tab-${index}`;
            const panelId = `${tabListId}-panel-${index}`;
            return (
              <div
                key={session.process_id}
                className={active ? styles.tabActive : styles.tab}
              >
                <button
                  type="button"
                  role="tab"
                  id={tabId}
                  aria-controls={panelId}
                  aria-selected={active}
                  className={styles.tabSelect}
                  onClick={() => activateSession(session.process_id)}
                  onKeyDown={(event) =>
                    handleTabKeyDown(event, session.process_id)
                  }
                  onFocus={() => setFocusedProcessId(session.process_id)}
                  ref={(node) => {
                    if (node) tabRefs.current.set(session.process_id, node);
                    else tabRefs.current.delete(session.process_id);
                  }}
                  tabIndex={
                    session.process_id ===
                    (focusedProcessId ??
                      activeProcessId ??
                      sessions[0]?.process_id)
                      ? 0
                      : -1
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
        <IconButton
          icon={workbenchOpen ? ChevronDown : ChevronUp}
          aria-label={
            workbenchOpen
              ? "Collapse terminal workbench"
              : "Expand terminal workbench"
          }
          size="sm"
          variant="plain"
          onClick={() =>
            dispatch(setTerminalWorkbenchOpen({ chatId, open: !workbenchOpen }))
          }
        />
      </header>

      <div className="rf-expand-grid" data-open={workbenchOpen}>
        <div
          className={styles.body}
          hidden={!workbenchOpen}
          aria-hidden={!workbenchOpen}
        >
          {disabled ? (
            <div className={styles.fullState}>
              <EmptyState
                icon={SquareTerminal}
                title="Browser terminal disabled"
                description="Terminal access is disabled by the daemon or REFACT_DISABLE_EXEC_HTTP policy. Enable exec HTTP access and try again."
                variant="full"
                action={
                  <Button
                    onClick={() => setListAttempt((attempt) => attempt + 1)}
                  >
                    Try again
                  </Button>
                }
              />
            </div>
          ) : (
            <>
              {sessions.map((session, index) => {
                const active = session.process_id === activeProcessId;
                return (
                  <div
                    key={session.process_id}
                    id={`${tabListId}-panel-${index}`}
                    aria-labelledby={`${tabListId}-tab-${index}`}
                    className={
                      active ? styles.sessionActive : styles.sessionHidden
                    }
                    hidden={!active}
                    role="tabpanel"
                    tabIndex={0}
                  >
                    {active && workbenchOpen ? (
                      <TerminalSession
                        processId={session.process_id}
                        chatId={chatId}
                        apiKey={apiKey}
                        focusRequest={terminalFocusRequest}
                        onStatusChange={handleStatusChange}
                        onResize={handleSessionResize}
                      />
                    ) : null}
                  </div>
                );
              })}
              {!loading && sessions.length === 0 ? (
                <EmptyState
                  icon={SquareTerminal}
                  title="No terminal sessions"
                  description="Start an interactive shell in this chat's workspace."
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
              {error ? (
                <div className={styles.panelError} role="alert">
                  {error}
                </div>
              ) : null}
            </>
          )}
        </div>
      </div>
    </section>
  );
}
