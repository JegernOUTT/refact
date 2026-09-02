import "@xterm/xterm/css/xterm.css";

import classNames from "classnames";
import {
  ChevronDown,
  ChevronUp,
  Plus,
  Search,
  SquareTerminal,
  X,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";

import {
  Badge,
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
import {
  clampDrawerHeight,
  DRAWER_MIN_HEIGHT,
  selectChatWorkspaceRoot,
  selectWorkspaceDrawer,
  setDrawerHeight,
} from "../workspaceSlice";
import {
  activeSessionChanged,
  expiredSessionsPruned,
  isFinishedStatus,
  selectActiveTerminalProcessId,
  selectTerminalSessions,
  selectTerminalWorkbenchOpen,
  sessionAdded,
  sessionExpiresAt,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
  setTerminalWorkbenchOpen,
  terminalSessionFromProcess,
  type TerminalSessionMetadata,
} from "./terminalSlice";
import styles from "./TerminalPanel.module.css";

const DEFAULT_PTY_ROWS = 24;
const DEFAULT_PTY_COLS = 80;
const KEYBOARD_RESIZE_STEP_PX = 32;
const INTERACTIVE_SHELL_ENV = {
  TERM: "xterm-256color",
  COLORTERM: "truecolor",
  NO_COLOR: "",
};

type TerminalPanelStyle = CSSProperties & {
  "--rf-terminal-h": string;
};

function viewportHeight(): number {
  return typeof window === "undefined" ? Number.NaN : window.innerHeight;
}

function isRunning(status: ExecStatus): boolean {
  return status === "running" || status === "starting";
}

type SessionTone = "running" | "error" | "success" | "idle";

function sessionTone({
  status,
  exit_code: exitCode,
}: TerminalSessionMetadata): SessionTone {
  if (isRunning(status)) return "running";
  if (status === "failed" || status === "timed_out" || status === "killed") {
    return "error";
  }
  if (status === "exited" && typeof exitCode === "number") {
    return exitCode === 0 ? "success" : "error";
  }
  return "idle";
}

function sessionStatusLabel({
  status,
  exit_code: exitCode,
}: TerminalSessionMetadata): string {
  if (status === "exited" && typeof exitCode === "number") {
    return `exit ${exitCode}`;
  }
  return status.replace(/_/g, " ");
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
  const drawer = useAppSelector(selectWorkspaceDrawer);
  const [loading, setLoading] = useState(true);
  const [spawning, setSpawning] = useState(false);
  const [disabled, setDisabled] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [listAttempt, setListAttempt] = useState(0);
  const [dragging, setDragging] = useState(false);
  const lastFittedRef = useRef<{ rows: number; cols: number } | null>(null);
  const latestSessionsRef = useRef(sessions);
  const panelRef = useRef<HTMLElement>(null);
  const liveHeightRef = useRef(drawer.height);
  const dragCleanupRef = useRef<(() => void) | null>(null);
  const tabListId = useId();
  const tabsRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef(new Map<string, HTMLButtonElement>());
  const [tabsOverflow, setTabsOverflow] = useState({
    start: false,
    end: false,
  });
  const [focusedProcessId, setFocusedProcessId] = useState<string | null>(null);
  const [terminalFocusRequest, setTerminalFocusRequest] = useState(0);
  const [terminalSearchRequest, setTerminalSearchRequest] = useState(0);
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
  latestSessionsRef.current = sessions;
  const height = clampDrawerHeight(drawer.height, viewportHeight());
  liveHeightRef.current = height;
  const activeSession =
    sessions.find((session) => session.process_id === activeProcessId) ?? null;
  const readOnlyActive = activeSession?.tty === false;

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
        const now = Date.now();
        const reattachedSessions = response.processes.map((process) =>
          terminalSessionFromProcess({
            ...process,
            ended_at_ms:
              process.ended_at_ms ??
              (isFinishedStatus(process.status) ? now : undefined),
          }),
        );
        const reattachedProcessIds = new Set(
          reattachedSessions.map((session) => session.process_id),
        );
        dispatch(
          sessionsReattached({
            chatId,
            sessions: [
              ...reattachedSessions,
              ...latestSessionsRef.current.filter(
                (session) => !reattachedProcessIds.has(session.process_id),
              ),
            ],
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
          env: INTERACTIVE_SHELL_ENV,
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
          session: terminalSessionFromProcess({
            process_id: result.process_id,
            command_preview: result.command_preview,
            status: result.status,
            tty: true,
          }),
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
      const running = isRunning(status);
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
    (
      processId: string,
      status: ExecStatus,
      exitCode?: number | null,
      endedAtMs?: number | null,
    ) => {
      dispatch(
        sessionStatusChanged({
          chatId,
          processId,
          status,
          ...(exitCode === undefined ? {} : { exit_code: exitCode }),
          ...(endedAtMs === undefined ? {} : { ended_at_ms: endedAtMs }),
        }),
      );
    },
    [chatId, dispatch],
  );

  useEffect(() => {
    const keepProcessId = workbenchOpen ? activeProcessId : null;
    const deadlines = sessions
      .filter((session) => session.process_id !== keepProcessId)
      .map(sessionExpiresAt)
      .filter((expiresAt): expiresAt is number => expiresAt !== null);
    if (deadlines.length === 0) return;
    const delay = Math.max(0, Math.min(...deadlines) - Date.now());
    const timer = setTimeout(() => {
      dispatch(
        expiredSessionsPruned({ chatId, now: Date.now(), keepProcessId }),
      );
    }, delay);
    return () => clearTimeout(timer);
  }, [activeProcessId, chatId, dispatch, sessions, workbenchOpen]);

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

  useEffect(() => {
    if (!activeProcessId) return;
    tabRefs.current
      .get(activeProcessId)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeProcessId]);

  const updateTabsOverflow = useCallback(() => {
    const tabs = tabsRef.current;
    if (!tabs) return;
    const start = tabs.scrollLeft > 0;
    const end = tabs.scrollLeft + tabs.clientWidth < tabs.scrollWidth - 1;
    setTabsOverflow((previous) =>
      previous.start === start && previous.end === end
        ? previous
        : { start, end },
    );
  }, []);

  useEffect(() => {
    const tabs = tabsRef.current;
    if (!tabs) return;
    updateTabsOverflow();
    const observer = new ResizeObserver(updateTabsOverflow);
    observer.observe(tabs);
    for (const tab of tabs.children) observer.observe(tab);
    return () => observer.disconnect();
  }, [sessions, updateTabsOverflow]);

  const handleTabsWheel = useCallback(
    (event: ReactWheelEvent<HTMLDivElement>) => {
      if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      event.currentTarget.scrollLeft += event.deltaY;
    },
    [],
  );

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

  const commitHeight = useCallback(
    (next: number) => {
      const clamped = clampDrawerHeight(next, viewportHeight());
      liveHeightRef.current = clamped;
      dispatch(setDrawerHeight(clamped));
    },
    [dispatch],
  );

  const handleResizePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      const panel = panelRef.current;
      if (!panel) return;
      event.preventDefault();
      dragCleanupRef.current?.();
      const startY = event.clientY;
      const startHeight = liveHeightRef.current;
      setDragging(true);
      document.body.style.cursor = "row-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const next = clampDrawerHeight(
          startHeight + startY - moveEvent.clientY,
          viewportHeight(),
        );
        liveHeightRef.current = next;
        panel.style.setProperty("--rf-terminal-h", `${next}px`);
      };

      const detach = () => {
        dragCleanupRef.current = null;
        setDragging(false);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        window.removeEventListener("pointermove", handlePointerMove);
        window.removeEventListener("pointerup", handlePointerUp);
        window.removeEventListener("pointercancel", handlePointerUp);
      };

      const handlePointerUp = () => {
        detach();
        dispatch(setDrawerHeight(liveHeightRef.current));
      };

      dragCleanupRef.current = detach;
      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp);
      window.addEventListener("pointercancel", handlePointerUp);
    },
    [dispatch],
  );

  const handleResizeKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (event.key === "ArrowUp") {
        event.preventDefault();
        commitHeight(liveHeightRef.current + KEYBOARD_RESIZE_STEP_PX);
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        commitHeight(liveHeightRef.current - KEYBOARD_RESIZE_STEP_PX);
      }
    },
    [commitHeight],
  );

  useEffect(() => () => dragCleanupRef.current?.(), []);

  return (
    <section
      ref={panelRef}
      className={styles.panel}
      aria-label={`Terminal workbench for ${chatId}`}
      data-open={workbenchOpen}
      data-dragging={dragging || undefined}
      style={{ "--rf-terminal-h": `${height}px` } as TerminalPanelStyle}
    >
      {workbenchOpen ? (
        <div
          role="separator"
          aria-label="Resize terminal"
          aria-orientation="horizontal"
          aria-valuemin={DRAWER_MIN_HEIGHT}
          aria-valuenow={Math.round(height)}
          tabIndex={0}
          className={styles.splitter}
          data-dragging={dragging || undefined}
          onKeyDown={handleResizeKeyDown}
          onPointerDown={handleResizePointerDown}
        >
          <span className={styles.splitterHandle} />
        </div>
      ) : null}
      <header className={styles.header}>
        <div className={styles.title}>
          <Icon icon={SquareTerminal} size="sm" tone="muted" />
          <span className={styles.titleText}>Terminal</span>
        </div>
        <div
          ref={tabsRef}
          className={styles.tabs}
          role="tablist"
          aria-label="Terminal sessions"
          data-overflow-start={tabsOverflow.start || undefined}
          data-overflow-end={tabsOverflow.end || undefined}
          onScroll={updateTabsOverflow}
          onWheel={handleTabsWheel}
        >
          {sessions.map((session, index) => {
            const active = session.process_id === activeProcessId;
            const tone = sessionTone(session);
            const statusLabel = sessionStatusLabel(session);
            const tabId = `${tabListId}-tab-${index}`;
            const panelId = `${tabListId}-panel-${index}`;
            return (
              <div
                key={session.process_id}
                className={classNames(styles.tab, active && styles.tabActive)}
                data-tone={tone}
              >
                <button
                  type="button"
                  role="tab"
                  id={tabId}
                  aria-controls={panelId}
                  aria-selected={active}
                  aria-label={`${session.title}, ${statusLabel}`}
                  title={session.title}
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
                  <StatusDot status={tone} pulse={tone === "running"} />
                  <span className={styles.tabTitle}>{session.label}</span>
                  <span className={styles.tabStatus}>{statusLabel}</span>
                </button>
                <button
                  type="button"
                  aria-label={`Close ${session.title}`}
                  className={styles.closeButton}
                  onClick={() =>
                    void handleCloseSession(session.process_id, session.status)
                  }
                >
                  <Icon icon={X} size="sm" />
                </button>
              </div>
            );
          })}
        </div>
        {readOnlyActive ? (
          <Badge
            className={styles.readOnlyBadge}
            size="xs"
            tone="muted"
            variant="outline"
          >
            Read-only
          </Badge>
        ) : null}
        <div className={styles.actions}>
          {activeSession && workbenchOpen ? (
            <IconButton
              icon={Search}
              aria-label="Search terminal"
              size="sm"
              variant="plain"
              onClick={() => setTerminalSearchRequest((request) => request + 1)}
            />
          ) : null}
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
              dispatch(
                setTerminalWorkbenchOpen({ chatId, open: !workbenchOpen }),
              )
            }
          />
        </div>
      </header>

      <div className="rf-expand-grid" data-open={workbenchOpen}>
        <div
          className={styles.body}
          data-testid="terminal-workbench-body"
          hidden={!workbenchOpen}
          aria-hidden={!workbenchOpen}
        >
          {disabled ? (
            <div className={styles.fullState}>
              <EmptyState
                className={styles.emptyState}
                icon={SquareTerminal}
                title="Browser terminal disabled"
                description="Terminal access is disabled by the daemon or REFACT_DISABLE_EXEC_HTTP policy. Enable exec HTTP access and try again."
                variant="compact"
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
                        readOnly={session.tty === false}
                        focusRequest={terminalFocusRequest}
                        searchRequest={terminalSearchRequest}
                        onStatusChange={handleStatusChange}
                        onResize={handleSessionResize}
                      />
                    ) : null}
                  </div>
                );
              })}
              {!loading && sessions.length === 0 ? (
                <EmptyState
                  className={styles.emptyState}
                  icon={SquareTerminal}
                  title="No terminal sessions"
                  description="Start an interactive shell in this chat's workspace."
                  variant="compact"
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
                  <span className={styles.panelErrorText}>{error}</span>
                  <IconButton
                    icon={X}
                    aria-label="Dismiss terminal error"
                    size="sm"
                    variant="plain"
                    onClick={() => setError(null)}
                  />
                </div>
              ) : null}
            </>
          )}
        </div>
      </div>
    </section>
  );
}
