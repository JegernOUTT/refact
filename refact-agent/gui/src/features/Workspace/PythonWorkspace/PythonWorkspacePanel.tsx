import { useCallback, useEffect, useId, useMemo, useState } from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import {
  ChevronRight,
  Copy,
  Ellipsis,
  FileCode,
  FileOutput,
  LocateFixed,
  RefreshCw,
} from "lucide-react";
import classNames from "classnames";
import {
  Button,
  EmptyState,
  ErrorState,
  FieldText,
  Icon,
  IconButton,
  LoadingState,
  Menu,
  StatusDot,
  Tooltip,
} from "../../../components/ui";
import {
  useGetPythonWorkspaceQuery,
  type PythonWorkspace,
  type PythonWorkspaceDisplay,
} from "../../../services/refact/pythonWorkspace";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import {
  formatBytes,
  formatTimestamp,
} from "../../Performance/performanceFormatters";
import { formatRelativeTime } from "../../Dashboard/dateUtils";
import {
  revealScriptRun,
  truncateText,
  runSummary,
  runTitle,
  splitDisplayPath,
} from "../../../components/ChatContent/ToolCard/pythonDisplay";
import {
  DisplayBody,
  OpenFullDataButton,
} from "../../../components/ChatContent/ToolCard/ScriptDisplays";
import {
  chartData,
  displayKindView,
  displaySize,
  displayTitle,
} from "../../../components/ChatContent/ToolCard/scriptDisplayHelpers";
import { visibleText } from "../../../components/ChatContent/ToolCard/safeText";
import { ReadOnlyChip } from "../../../components/ChatContent/ToolCard/ReadOnlyChip";
import styles from "./PythonWorkspacePanel.module.css";

const RUNS_SHOWN = 5;
const DISPLAYS_SHOWN = 4;
const ARTIFACTS_SHOWN = 8;
const VARIABLES_SHOWN = 12;
const VARIABLES_OPEN_MAX = 12;
const VARIABLES_FILTER_MIN = 8;
const MINUTE_MS = 60_000;
const SECTIONS_KEY = "refact.pythonWorkspace.sections";
const ACTIONS_LABEL_MAX_CHARS = 80;
const SAFE_RUN_ID = /^[A-Za-z0-9_-]+$/;
const NOT_IN_CHAT = "This run's card is not loaded in the chat";

type Run = PythonWorkspace["runs"][number];
type SectionId = "session" | "runs" | "displays" | "artifacts" | "variables";

const SESSION_STATUS: Record<
  PythonWorkspace["session"]["status"],
  { label: string; dot: "idle" | "running" }
> = {
  none: { label: "No session yet", dot: "idle" },
  idle: { label: "Idle", dot: "idle" },
  running: { label: "Running", dot: "running" },
  background: { label: "Background", dot: "running" },
};

function isLive(run: Run) {
  return (
    run.finished_at_ms === null &&
    (run.status === "running" || run.status === "background")
  );
}

function relative(ms: number | null) {
  if (ms === null || ms <= 0) return "never";
  return formatRelativeTime(ms) || formatTimestamp(ms);
}

/** Session line: never claim a restore the engine did not report. */
function sessionLabel(session: PythonWorkspace["session"]) {
  if (session.live)
    return session.last_run_restored ? "Restored from disk" : "Live session";
  if (session.state_persisted) return "Saved on disk, not loaded";
  return "No saved state";
}

/** Re-renders once a minute so relative times stay current without a refetch. */
function useMinuteTick() {
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTick((tick) => tick + 1), MINUTE_MS);
    return () => clearInterval(timer);
  }, []);
}

function readStoredSections(): Partial<Record<SectionId, boolean>> {
  try {
    const raw = window.localStorage.getItem(SECTIONS_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    if (typeof parsed !== "object" || parsed === null) return {};
    const result: Partial<Record<SectionId, boolean>> = {};
    for (const [key, value] of Object.entries(parsed)) {
      if (typeof value === "boolean") result[key as SectionId] = value;
    }
    return result;
  } catch {
    return {};
  }
}

/** Open state of the panel sections, remembered across panel mounts. */
function useSectionState() {
  const [stored, setStored] = useState(readStoredSections);
  const toggle = useCallback((id: SectionId, current: boolean) => {
    setStored((previous) => {
      const next = { ...previous, [id]: !current };
      try {
        window.localStorage.setItem(SECTIONS_KEY, JSON.stringify(next));
      } catch {
        /* storage unavailable: keep the in-memory state */
      }
      return next;
    });
  }, []);
  return [stored, toggle] as const;
}

function Disclosure({
  id,
  title,
  count,
  open,
  onToggle,
  children,
}: {
  id: SectionId;
  title: string;
  count?: number;
  open: boolean;
  onToggle: (id: SectionId, current: boolean) => void;
  children: React.ReactNode;
}) {
  const bodyId = useId();
  const label = count === undefined ? title : `${title} · ${count}`;
  return (
    <section className={styles.section} aria-label={title}>
      <h3 className={styles.sectionHeading}>
        <button
          type="button"
          className={styles.sectionToggle}
          aria-expanded={open}
          aria-controls={open ? bodyId : undefined}
          onClick={() => onToggle(id, open)}
        >
          <ChevronRight
            aria-hidden
            className={classNames(styles.chevron, open && styles.chevronOpen)}
          />
          <span>{label}</span>
        </button>
      </h3>
      {open && (
        <div id={bodyId} className={styles.sectionBody}>
          {children}
        </div>
      )}
    </section>
  );
}

function ShowAll({
  hidden,
  label,
  onClick,
}: {
  hidden: number;
  label: string;
  onClick: () => void;
}) {
  if (hidden <= 0) return null;
  return (
    <button type="button" className={styles.linkButton} onClick={onClick}>
      {label}
    </button>
  );
}

function RunRow({
  run,
  workspaceDir,
  canOpen,
  openFile,
  copy,
}: {
  run: Run;
  workspaceDir: string;
  canOpen: boolean;
  openFile: (target: { path: string }) => void;
  copy: (text: string) => void;
}) {
  const title = runTitle(run);
  const summary = runSummary(run);
  const runDir = `${workspaceDir}/runs/${run.run_id}`;
  const [inChat, setInChat] = useState(() => hasRunCard(run.run_id));
  const recheck = () => setInChat(hasRunCard(run.run_id));
  useEffect(() => {
    setInChat(hasRunCard(run.run_id));
  }, [run]);
  const reasonId = useId();
  const openable =
    canOpen && workspaceDir !== "" && SAFE_RUN_ID.test(run.run_id);
  return (
    <li className={styles.runRow}>
      <button
        type="button"
        className={styles.runMain}
        aria-disabled={!inChat || undefined}
        aria-describedby={inChat ? undefined : reasonId}
        title={inChat ? "Show in chat" : NOT_IN_CHAT}
        onPointerEnter={recheck}
        onFocus={recheck}
        onClick={() => setInChat(revealScriptRun(run.run_id))}
      >
        <span
          className={classNames(styles.runTitle, title.mono && styles.mono)}
        >
          {title.text}
        </span>
        <span className={styles.runMeta}>
          {summary.lead} ·{" "}
          <time title={formatTimestamp(run.started_at_ms)}>
            {relative(run.started_at_ms)}
          </time>
          {summary.tail && ` · ${summary.tail}`}
        </span>
        {!inChat && (
          <span id={reasonId} className={styles.srOnly}>
            {NOT_IN_CHAT}
          </span>
        )}
      </button>
      {run.read_only && <ReadOnlyChip />}
      <Menu onOpenChange={(open) => open && recheck()}>
        <Menu.Trigger asChild>
          <IconButton
            aria-label={`Actions for ${truncateText(
              title.text,
              ACTIONS_LABEL_MAX_CHARS,
            )}`}
            icon={Ellipsis}
            size="sm"
            variant="ghost"
            className={styles.runMenu}
          />
        </Menu.Trigger>
        <Menu.Content align="end">
          <Menu.Item
            disabled={!inChat}
            onSelect={() => setInChat(revealScriptRun(run.run_id))}
          >
            <Icon icon={LocateFixed} size="sm" />
            Show in chat
          </Menu.Item>
          {openable && (
            <Menu.Item onSelect={() => openFile({ path: `${runDir}/code.py` })}>
              <Icon icon={FileCode} size="sm" />
              Open code
            </Menu.Item>
          )}
          {openable && !isLive(run) && (
            <Menu.Item
              onSelect={() => openFile({ path: `${runDir}/stdout.txt` })}
            >
              <Icon icon={FileOutput} size="sm" />
              Open output
            </Menu.Item>
          )}
          <Menu.Item onSelect={() => copy(run.run_id)}>
            <Icon icon={Copy} size="sm" />
            Copy run ID
          </Menu.Item>
        </Menu.Content>
      </Menu>
    </li>
  );
}

function hasRunCard(runId: string) {
  return Array.from(document.querySelectorAll("[data-python-run-id]")).some(
    (node) => node.getAttribute("data-python-run-id") === runId,
  );
}

function ActionIcon({
  label,
  icon,
  onClick,
}: {
  label: string;
  icon: React.ComponentProps<typeof IconButton>["icon"];
  onClick: () => void;
}) {
  return (
    <Tooltip content={label}>
      <IconButton
        aria-label={label}
        icon={icon}
        size="sm"
        variant="ghost"
        onClick={onClick}
      />
    </Tooltip>
  );
}

function DisplayRow({
  display,
  workspaceDir,
}: {
  display: PythonWorkspaceDisplay;
  workspaceDir: string;
}) {
  const [open, setOpen] = useState(false);
  const [inChat, setInChat] = useState(() => hasRunCard(display.run_id));
  const recheck = () => setInChat(hasRunCard(display.run_id));
  const bodyId = useId();
  const reasonId = useId();
  const kind = displayKindView(display.kind);
  const title = visibleText(displayTitle(display));
  const size = displaySize(display);
  const data = useMemo(() => chartData(display), [display]);
  return (
    <li className={styles.display}>
      <button
        type="button"
        className={styles.displayRow}
        aria-expanded={open}
        aria-controls={open ? bodyId : undefined}
        onClick={() => setOpen((value) => !value)}
      >
        <ChevronRight
          aria-hidden
          className={classNames(styles.chevron, open && styles.chevronOpen)}
        />
        <Icon icon={kind.icon} size="sm" tone="muted" />
        <span className={styles.displayText}>
          <span className={styles.displayTitle}>{title}</span>
          <span className={styles.displayMeta}>
            {size && `${size} · `}
            <time title={formatTimestamp(display.started_at_ms)}>
              {relative(display.started_at_ms)}
            </time>
          </span>
        </span>
      </button>
      {open && (
        <div id={bodyId} className={styles.displayBody}>
          <DisplayBody display={display} data={data} density="rail" />
          <div className={styles.displayActions}>
            <Button
              size="sm"
              variant="ghost"
              leftIcon={LocateFixed}
              aria-disabled={!inChat || undefined}
              aria-describedby={inChat ? undefined : reasonId}
              title={inChat ? undefined : NOT_IN_CHAT}
              onPointerEnter={recheck}
              onFocus={recheck}
              onClick={() => setInChat(revealScriptRun(display.run_id))}
            >
              Show in chat
            </Button>
            {!inChat && (
              <span id={reasonId} className={styles.srOnly}>
                {NOT_IN_CHAT}
              </span>
            )}
            <OpenFullDataButton
              display={display}
              title={title}
              baseDir={workspaceDir || undefined}
            />
          </div>
        </div>
      )}
    </li>
  );
}

function Displays({
  displays,
  truncated,
  workspaceDir,
}: {
  displays: PythonWorkspaceDisplay[];
  truncated: boolean;
  workspaceDir: string;
}) {
  const [all, setAll] = useState(false);
  const shown = all ? displays : displays.slice(0, DISPLAYS_SHOWN);
  return (
    <>
      <ul className={styles.list}>
        {shown.map((display) => (
          <DisplayRow
            key={`${display.run_id}:${display.index}`}
            display={display}
            workspaceDir={workspaceDir}
          />
        ))}
      </ul>
      <ShowAll
        hidden={displays.length - shown.length}
        label={`Show all ${displays.length} displays`}
        onClick={() => setAll(true)}
      />
      {truncated && <p className={styles.meta}>Older displays not shown</p>}
    </>
  );
}

function VariableRow({
  variable,
}: {
  variable: PythonWorkspace["session"]["variables"][number];
}) {
  const [open, setOpen] = useState(false);
  const detailId = useId();
  return (
    <li className={styles.variable}>
      <button
        type="button"
        className={styles.variableRow}
        aria-expanded={open}
        aria-controls={open ? detailId : undefined}
        onClick={() => setOpen((value) => !value)}
      >
        <code className={styles.variableName}>{variable.name}</code>
        <span className={styles.variableType}>{variable.type_name}</span>
        <code className={styles.variablePreview}>{variable.preview}</code>
      </button>
      {open && (
        <pre id={detailId} className={styles.variableDetail}>
          {variable.preview}
        </pre>
      )}
    </li>
  );
}

function Variables({ session }: { session: PythonWorkspace["session"] }) {
  const [filter, setFilter] = useState("");
  const [all, setAll] = useState(false);
  const loaded = session.variables;
  const total = session.variables_total ?? loaded.length;
  const filterable = loaded.length > VARIABLES_FILTER_MIN || filter !== "";
  const query = filterable ? filter.trim().toLowerCase() : "";
  const matching = useMemo(
    () =>
      query
        ? loaded.filter(
            (variable) =>
              variable.name.toLowerCase().includes(query) ||
              variable.type_name.toLowerCase().includes(query),
          )
        : loaded,
    [loaded, query],
  );
  const shown = all ? matching : matching.slice(0, VARIABLES_SHOWN);
  const capped = session.variables_truncated || total > loaded.length;
  const unknown = session.variables_unknown === true;
  if (loaded.length === 0)
    return (
      <p className={styles.meta}>
        {unknown ? "Variables could not be listed" : "No variables"}
      </p>
    );
  return (
    <>
      {unknown && (
        <p className={styles.meta}>Some variables could not be listed</p>
      )}
      {filterable && (
        <FieldText
          aria-label="Filter variables"
          placeholder="Filter variables"
          className={styles.filter}
          type="search"
          value={filter}
          onChange={setFilter}
        />
      )}
      {capped && (
        <p className={styles.meta}>
          {total > loaded.length
            ? `${loaded.length} of ${total} variables loaded. Filtering searches loaded variables only.`
            : "More variables not loaded. Filtering searches loaded variables only."}
        </p>
      )}
      {matching.length === 0 ? (
        <p className={styles.meta}>No matching variables</p>
      ) : (
        <ul className={styles.list}>
          {shown.map((variable) => (
            <VariableRow key={variable.name} variable={variable} />
          ))}
        </ul>
      )}
      <ShowAll
        hidden={matching.length - shown.length}
        label={`Show all ${matching.length} loaded variables`}
        onClick={() => setAll(true)}
      />
    </>
  );
}

export function PythonWorkspacePanel({ chatId }: { chatId: string | null }) {
  const {
    currentData: data,
    isLoading,
    isError,
    isFetching,
    refetch,
  } = useGetPythonWorkspaceQuery(chatId ? { chatId } : skipToken, {
    refetchOnMountOrArgChange: true,
  });
  useMinuteTick();
  const { openFile, canOpen } = useOpenFileInApp();
  const copy = useCopyToClipboard();
  const [stored, toggle] = useSectionState();
  const [allRuns, setAllRuns] = useState(false);
  const [allArtifacts, setAllArtifacts] = useState(false);
  useEffect(() => {
    if (!chatId) return;
    const refresh = () => {
      void refetch();
    };
    window.addEventListener("focus", refresh);
    return () => window.removeEventListener("focus", refresh);
  }, [chatId, refetch]);
  const runs = useMemo(
    () =>
      data
        ? [...data.runs].sort((a, b) => b.started_at_ms - a.started_at_ms)
        : [],
    [data],
  );
  const isOpen = (id: SectionId, fallback: boolean) => stored[id] ?? fallback;
  const empty =
    data &&
    (!data.exists ||
      (runs.length === 0 &&
        data.artifacts.length === 0 &&
        data.session.variables.length === 0 &&
        data.session.status === "none"));
  return (
    <section className={styles.panel} aria-label="Python workspace">
      <header className={styles.top}>
        <h2 className={styles.heading}>Python workspace</h2>
        <IconButton
          aria-label="Refresh"
          icon={RefreshCw}
          size="sm"
          variant="ghost"
          disabled={!chatId || isFetching}
          onClick={() => {
            void refetch();
          }}
        />
      </header>
      <div className={styles.body} aria-busy={isFetching || undefined}>
        {!chatId ? (
          <EmptyState title="Focus a chat to see its script workspace" />
        ) : isLoading ? (
          <LoadingState label="Loading workspace…" />
        ) : isError && !data ? (
          <ErrorState
            title="Couldn't load workspace"
            retry={
              <Button
                size="sm"
                variant="soft"
                onClick={() => {
                  void refetch();
                }}
              >
                Try again
              </Button>
            }
          />
        ) : !data ? null : empty ? (
          <EmptyState
            title="No script runs yet"
            description="This chat's session, runs and artifacts will appear here."
          />
        ) : (
          <>
            <Disclosure
              id="session"
              title="Session"
              open={isOpen("session", true)}
              onToggle={toggle}
            >
              <Session data={data} runs={runs} />
            </Disclosure>
            <Disclosure
              id="runs"
              title="Recent runs"
              count={runs.length}
              open={isOpen("runs", true)}
              onToggle={toggle}
            >
              {runs.length === 0 ? (
                <p className={styles.meta}>No recent runs</p>
              ) : (
                <ul className={styles.list}>
                  {(allRuns ? runs : runs.slice(0, RUNS_SHOWN)).map((run) => (
                    <RunRow
                      key={run.run_id}
                      run={run}
                      workspaceDir={data.workspace_dir}
                      canOpen={canOpen}
                      openFile={openFile}
                      copy={copy}
                    />
                  ))}
                </ul>
              )}
              <ShowAll
                hidden={allRuns ? 0 : runs.length - RUNS_SHOWN}
                label={`Show all ${runs.length} runs`}
                onClick={() => setAllRuns(true)}
              />
              {data.runs_truncated && (
                <p className={styles.meta}>Older runs not shown</p>
              )}
            </Disclosure>
            {data.displays && data.displays.length > 0 && (
              <Disclosure
                id="displays"
                title="Displays"
                count={data.displays.length}
                open={isOpen("displays", true)}
                onToggle={toggle}
              >
                <Displays
                  displays={data.displays}
                  truncated={data.displays_truncated ?? false}
                  workspaceDir={data.workspace_dir}
                />
              </Disclosure>
            )}
            <Disclosure
              id="artifacts"
              title="Artifacts"
              count={data.artifacts.length}
              open={isOpen("artifacts", true)}
              onToggle={toggle}
            >
              {data.artifacts.length === 0 ? (
                <p className={styles.meta}>No artifacts</p>
              ) : (
                <ul className={styles.list}>
                  {(allArtifacts
                    ? data.artifacts
                    : data.artifacts.slice(0, ARTIFACTS_SHOWN)
                  ).map((artifact, index) => {
                    const { name, dir } = splitDisplayPath(
                      artifact.rel_path,
                      undefined,
                    );
                    return (
                      <li
                        key={`${index}:${artifact.path}`}
                        className={styles.fileRow}
                      >
                        <div className={styles.fileText}>
                          <span className={styles.path} title={artifact.path}>
                            <code className={styles.pathName}>{name}</code>
                            {dir && (
                              <span className={styles.pathDir}>{dir}</span>
                            )}
                          </span>
                          <span className={styles.meta}>
                            {formatBytes(artifact.bytes)} ·{" "}
                            <time
                              title={formatTimestamp(artifact.modified_at_ms)}
                            >
                              {relative(artifact.modified_at_ms)}
                            </time>
                          </span>
                        </div>
                        <div className={styles.rowActions}>
                          {canOpen && (
                            <Button
                              size="sm"
                              variant="ghost"
                              aria-label={`Open ${name}`}
                              onClick={() => openFile({ path: artifact.path })}
                            >
                              Open
                            </Button>
                          )}
                          <ActionIcon
                            label={`Copy path of ${name}`}
                            icon={Copy}
                            onClick={() => copy(artifact.path)}
                          />
                        </div>
                      </li>
                    );
                  })}
                </ul>
              )}
              <ShowAll
                hidden={
                  allArtifacts ? 0 : data.artifacts.length - ARTIFACTS_SHOWN
                }
                label={`Show all ${data.artifacts.length}`}
                onClick={() => setAllArtifacts(true)}
              />
              {data.artifacts_truncated && (
                <p className={styles.meta}>More artifacts not shown</p>
              )}
            </Disclosure>
            <Disclosure
              id="variables"
              title="Variables"
              count={
                data.session.variables_unknown
                  ? undefined
                  : data.session.variables_total ??
                    data.session.variables.length
              }
              open={isOpen(
                "variables",
                (data.session.variables_total ??
                  data.session.variables.length) <= VARIABLES_OPEN_MAX,
              )}
              onToggle={toggle}
            >
              <Variables session={data.session} />
            </Disclosure>
            <div className={styles.storage}>
              <span className={styles.meta}>
                Storage · {formatBytes(data.disk.workspace_bytes)} of{" "}
                {formatBytes(data.disk.quota_bytes)}
              </span>
              <progress
                className={styles.disk}
                aria-label="Workspace storage usage"
                max={data.disk.quota_bytes || 1}
                value={Math.min(
                  data.disk.workspace_bytes,
                  data.disk.quota_bytes,
                )}
              />
            </div>
          </>
        )}
      </div>
    </section>
  );
}

function Session({ data, runs }: { data: PythonWorkspace; runs: Run[] }) {
  const session = data.session;
  const status = SESSION_STATUS[session.status];
  const liveRun = runs.find(isLive);
  const liveTitle = liveRun ? runTitle(liveRun) : undefined;
  return (
    <>
      <div className={styles.line}>
        <StatusDot status={status.dot} pulse={status.dot === "running"} />
        <span>{status.label}</span>
        <span className={styles.meta}>{sessionLabel(session)}</span>
      </div>
      {session.state_persisted && (
        <p className={styles.meta}>
          Saved{" "}
          <time
            title={
              session.saved_at_ms
                ? formatTimestamp(session.saved_at_ms)
                : undefined
            }
          >
            {relative(session.saved_at_ms)}
          </time>{" "}
          · {formatBytes(session.state_bytes)}
        </p>
      )}
      {session.reset_reason && (
        <p className={styles.meta}>Reset: {session.reset_reason}</p>
      )}
      {liveRun && liveTitle && (
        <div className={styles.liveRun}>
          <span
            className={classNames(
              styles.runTitle,
              liveTitle.mono && styles.mono,
            )}
            title={liveTitle.text}
          >
            {liveTitle.text}
          </span>
          <Button
            size="sm"
            variant="ghost"
            leftIcon={LocateFixed}
            onClick={() => revealScriptRun(liveRun.run_id)}
          >
            Show in chat
          </Button>
        </div>
      )}
    </>
  );
}
