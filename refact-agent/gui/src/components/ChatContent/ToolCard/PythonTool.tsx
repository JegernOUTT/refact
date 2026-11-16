import {
  memo,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import classNames from "classnames";
import {
  ChevronsDownUp,
  ChevronsUpDown,
  Copy,
  FileCode,
  FileOutput,
  FlaskConical,
  SquareCode,
  Square,
} from "lucide-react";
import { useToolCallActivity } from "./useToolCallActivity";
import { ToolJobResultsContext } from "./ToolJobResultsContext";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import {
  selectPythonApprovals,
  selectPythonRun,
} from "../../../features/Chat/Thread/selectors";
import {
  selectApiKey,
  selectConfig,
} from "../../../features/Config/configSlice";
import {
  setDockOpen,
  setDockSection,
} from "../../../features/Workspace/workspaceSlice";
import {
  normalizePythonRunExtra,
  type DiffMessage,
  type PythonCallRecord,
  type PythonDisplay,
  type PythonLlmTotals,
  type PythonRunExtra,
  type ToolCall,
  type ToolResult,
} from "../../../services/refact/types";
import {
  isTerminalToolJob,
  parseToolJobReceipt,
} from "../../../services/refact/toolJobs";
import {
  ChatCommandHttpError,
  sendPythonRunControl,
} from "../../../services/refact/chatCommands";
import { formatBytes } from "../../../features/Performance/performanceFormatters";
import { Button, IconButton } from "../../ui";
import {
  TranscriptFacts,
  TranscriptItem,
  type TranscriptFact,
  type TranscriptRawBlock,
} from "../TranscriptItem";
import { useUnrecordedResultState } from "../TranscriptItem/useUnrecordedResultState";
import { useCollapsibleStore, useStoredOpen } from "../useStoredOpen";
import { GroupedDiffs } from "../DiffContent";
import { NestedToolCalls } from "../NestedToolCalls";
import { ClampedText, ScriptCodeView } from "./PythonBlocks";
import { ScriptDisplays } from "./ScriptDisplays";
import {
  HELPER_CALLEES,
  callCounts,
  compactCount,
  formatCount,
  isModelCall,
} from "./scriptDisplayHelpers";
import { EngineChangesContext, engineChangeCount } from "./engineChanges";
import { EngineChangesSection } from "./EngineChangesSection";
import { ShikiCodeBlock } from "../../Markdown";
import { selectChatWorkspaceRoots } from "./pythonRoots";
import {
  byteLength,
  codePointBoundary,
  formatCallDuration,
  prettyJson,
  limitSentence,
  placeCalls,
  plural,
  scriptArgs,
  scriptArtifacts,
  scriptChanges,
  scriptStatusView,
  scriptTitle,
  sessionSummary,
  splitCodeLines,
  splitDisplayPath,
  type ScriptChange,
  type WorkspaceRoots,
} from "./pythonDisplay";
import styles from "./PythonTool.module.css";

const TRACEBACK_LINES = 20;
const PREVIEW_MAX_CHARS = 20_000;
/** JSON results up to this size are highlighted; larger ones stay plain text. */
const JSON_HIGHLIGHT_MAX_CHARS = 4_000;
/** The live stdout tail the thread keeps (`pythonStdoutTail` in the reducer). */
const LIVE_STDOUT_TAIL_BYTES = 16_384;
/** The tail may start a few bytes later, on a UTF-8 boundary. */
const LIVE_STDOUT_TAIL_NEAR = LIVE_STDOUT_TAIL_BYTES - 3;
const MAX_CHANGED_FILES = 5;
const HIDDEN_LABELS = { output: "", actions: "" };
const EMPTY_CALLS: PythonCallRecord[] = [];
const EMPTY_DISPLAYS: PythonDisplay[] = [];

export const BACKGROUND_NOTE =
  "Running in the background. The result appears here when the run finishes.";
export const WAITING_NOTE =
  "A call is waiting for your approval in the bar below.";
/** A Stop request re-enables the button if the run is still going after this. */
export const STOP_RETRY_MS = 10_000;

function deliveredExtra(
  jobs: React.ContextType<typeof ToolJobResultsContext>,
  toolCallId: string,
  result: ToolResult | undefined,
) {
  const delivered = (jobs[toolCallId] ?? [])
    .map((job) => job.message)
    .filter((message) => message.role === "tool")
    .reverse();
  for (const message of delivered) {
    const normalized = normalizePythonRunExtra(message.extra?.python);
    if (normalized)
      return {
        extra: normalized,
        rawContent: typeof message.content === "string" ? message.content : "",
      };
  }
  return {
    extra: normalizePythonRunExtra(result?.extra?.python),
    rawContent: typeof result?.content === "string" ? result.content : "",
  };
}

export function PythonTool({
  toolCall,
  result: suppliedResult,
}: {
  toolCall: ToolCall;
  result?: ToolResult;
}) {
  const activity = useToolCallActivity(toolCall.id);
  const chatId = activity.threadId;
  const toolCallId = toolCall.id ?? "";
  const run = useAppSelector((state) =>
    selectPythonRun(state, chatId, toolCallId),
  );
  const approvals = useAppSelector((state) =>
    selectPythonApprovals(state, chatId),
  );
  const roots = useAppSelector((state) =>
    selectChatWorkspaceRoots(state, chatId),
  );
  const connection = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const jobs = useContext(ToolJobResultsContext);
  const unrecorded = useUnrecordedResultState(toolCall.id, chatId);
  const result = suppliedResult ?? activity.result;
  const { extra, rawContent } = useMemo(
    () => deliveredExtra(jobs, toolCallId, result),
    [jobs, toolCallId, result],
  );
  const receipt = parseToolJobReceipt(result?.extra?.tool_job);
  const receiptPending =
    !extra && receipt !== null && !isTerminalToolJob(receipt.status);
  const busy =
    !extra && (run?.status === "running" || run?.status === "background");
  const ownApprovals = useMemo(
    () => approvals.filter((request) => request.tool_call_id === toolCallId),
    [approvals, toolCallId],
  );
  const waitingCallIds = useMemo(
    () => new Set(ownApprovals.map((request) => request.call_id)),
    [ownApprovals],
  );
  const waiting = !extra && waitingCallIds.size > 0;
  const statusKey = runStatusKey({
    waiting,
    extra,
    run,
    receipt: receipt !== null,
    receiptPending,
    result,
  });
  const view = statusKey ? scriptStatusView(statusKey) : undefined;
  const transcriptStatus = view?.transcript ?? unrecorded.status;
  const statusLabel = view?.label ?? unrecorded.statusLabel;

  const {
    code,
    description,
    cut: codeCut,
  } = useMemo(
    () => scriptArgs(toolCall.function.arguments),
    [toolCall.function.arguments],
  );
  const title = useMemo(
    () =>
      scriptTitle({
        description,
        storedDescription: extra?.description,
        code,
        codePreview: extra?.code_preview,
      }),
    [code, description, extra?.code_preview, extra?.description],
  );
  const bodyId = useId();
  const sawLive = useRef(false);
  if (busy || receiptPending || waiting) sawLive.current = true;

  const [open, , setOpen] = useStoredOpen(
    toolCall.id ? `tc:${toolCall.id}` : undefined,
    false,
  );
  const [stopping, setStopping] = useState(false);
  const lock = useRef(false);
  const [stopError, setStopError] = useState<string>();
  useEffect(() => {
    lock.current = false;
    setStopping(false);
    if (!busy) setStopError(undefined);
  }, [run?.run_id, run?.status, busy]);
  useEffect(() => {
    if (!stopping || !busy) return;
    const timer = setTimeout(() => {
      lock.current = false;
      setStopping(false);
    }, STOP_RETRY_MS);
    return () => clearTimeout(timer);
  }, [stopping, busy]);
  const stop = async () => {
    if (!toolCall.id || lock.current) return;
    lock.current = true;
    setStopping(true);
    setStopError(undefined);
    try {
      await sendPythonRunControl(
        chatId,
        toolCall.id,
        "stop",
        connection,
        apiKey ?? undefined,
      );
    } catch (cause) {
      setStopError(
        cause instanceof ChatCommandHttpError && cause.status === 400
          ? "This script is no longer running."
          : "Could not stop this script. Please try again.",
      );
      setOpen(true);
      lock.current = false;
      setStopping(false);
    }
  };

  const [expanded, toggleExpanded] = useStoredOpen(
    toolCall.id ? `script:${toolCall.id}` : undefined,
    false,
  );
  const calls = useMemo(
    () => extra?.calls ?? run?.calls ?? EMPTY_CALLS,
    [extra?.calls, run?.calls],
  );
  const artifactsDir = extra?.workspace_dir
    ? `${extra.workspace_dir}/artifacts`
    : undefined;
  const changes = useMemo(
    () =>
      scriptChanges(
        { calls, files_written: extra?.files_written ?? [] },
        undefined,
        artifactsDir,
      ),
    [artifactsDir, calls, extra?.files_written],
  );
  const callsTotal = extra?.calls_total ?? calls.length;
  const counts = callCounts(calls, callsTotal, extra?.llm?.calls);
  const displays = extra ? extra.displays ?? EMPTY_DISPLAYS : run?.displays;
  const shownDisplays = displays ?? EMPTY_DISPLAYS;
  const scriptDisplays =
    shownDisplays.length > 0 || (extra?.displays_total ?? 0) > 0 ? (
      <ScriptDisplays
        displays={shownDisplays}
        total={extra?.displays_total}
        baseDir={extra?.workspace_dir ? extra.workspace_dir : undefined}
        runKey={toolCallId || undefined}
      />
    ) : null;
  const started =
    extra?.started_at_ms ??
    (run && run.status !== "finished" ? run.started_at_ms : undefined);
  const background = extra?.background ?? run?.background ?? receipt !== null;
  const live = busy || receiptPending;
  const engineChanges = extra?.engine_changes;
  const engineCount = engineChangeCount(
    engineChanges,
    extra?.engine_changes_total,
    calls,
    extra !== undefined,
  );
  const filesPart = changes.length > 0 ? plural(changes.length, "file") : null;
  const isPart = (part: string | null): part is string => part !== null;
  const metaParts = [
    counts.tools > 0 ? plural(counts.tools, "call") : null,
    counts.models > 0 ? plural(counts.models, "model call") : null,
    filesPart,
    engineCount > 0 ? plural(engineCount, "engine change") : null,
  ].filter(isPart);
  // Narrow cards: "3 + 13 model · 2 changes" keeps every kind and leaves room for the title.
  const narrowParts =
    (counts.tools > 0 && counts.models > 0) || engineCount > 0
      ? [
          counts.tools > 0 && counts.models > 0
            ? `${counts.tools} + ${counts.models} model`
            : metaParts.find((part) => /calls?$/.test(part)) ?? null,
          filesPart,
          engineCount > 0 ? plural(engineCount, "change") : null,
        ].filter(isPart)
      : undefined;
  const meta = (
    <>
      {narrowParts ? (
        <>
          <span className={styles.srOnly}>{metaParts.join(" · ")}</span>
          <span className={styles.metaWide} aria-hidden>
            {metaParts.join(" · ")}
          </span>
          <span
            className={styles.metaNarrow}
            title={metaParts.join(" · ")}
            aria-hidden
          >
            {narrowParts.join(" · ")}
          </span>
        </>
      ) : (
        metaParts.join(" · ")
      )}
      {started !== undefined && (
        <>
          {metaParts.length > 0 && " · "}
          <Elapsed
            started={started}
            finished={extra?.finished_at_ms}
            busy={busy}
          />
        </>
      )}
      {background && !live && (extra ?? run) && (
        <>
          {metaParts.length > 0 || started !== undefined ? " · " : ""}ran in
          background
        </>
      )}
    </>
  );
  const toolNames = useMemo(
    () =>
      new Set([
        ...HELPER_CALLEES,
        ...calls.map((call) => call.tool),
        ...ownApprovals.map((request) => request.tool_name),
      ]),
    [calls, ownApprovals],
  );

  return (
    <TranscriptItem
      data-testid="script-card"
      data-python-run-id={extra?.run_id ?? run?.run_id}
      aria-label={`Script: ${title.text}`}
      icon={<SquareCode />}
      verb="Script"
      target={
        <span
          className={classNames(title.mono && styles.monoTitle)}
          title={title.text}
        >
          {title.text}
        </span>
      }
      meta={meta}
      status={transcriptStatus}
      statusLabel={transcriptStatus === "done" ? undefined : statusLabel}
      rowClassName={styles.scriptRow}
      quickAction={
        busy || open ? (
          <>
            {busy && (
              <Button
                leftIcon={Square}
                size="sm"
                variant="ghost"
                disabled={stopping}
                onClick={() => void stop()}
              >
                {stopping ? "Stopping…" : "Stop"}
              </Button>
            )}
            {open && (
              <IconButton
                icon={expanded ? ChevronsDownUp : ChevronsUpDown}
                size="sm"
                variant="ghost"
                className={styles.detailsToggle}
                aria-expanded={expanded}
                aria-controls={bodyId}
                aria-label="Details"
                onClick={toggleExpanded}
              />
            )}
          </>
        ) : null
      }
      open={open}
      onOpenChange={setOpen}
      preview={open ? null : scriptDisplays}
      sectionLabels={HIDDEN_LABELS}
      output={
        <ScriptBody
          id={bodyId}
          code={code}
          codeCut={codeCut}
          announce={sawLive.current}
          extra={extra}
          rawContent={rawContent}
          liveStdout={run?.stdout ?? ""}
          calls={calls}
          waitingCallIds={waitingCallIds}
          toolNames={toolNames}
          waiting={waiting}
          receipt={receipt !== null}
          receiptPending={receiptPending}
          busy={busy}
          backgroundLive={busy && run.status === "background"}
          stopError={stopError}
          changes={changes}
          artifactsDir={artifactsDir}
          roots={roots}
          expanded={expanded}
          onToggleExpanded={toggleExpanded}
          toolCallId={toolCallId}
          displays={open ? scriptDisplays : null}
        />
      }
      actions={
        expanded ? (
          <ScriptActions
            code={code}
            codeCut={codeCut}
            extra={extra}
            stdout={extra?.stdout_preview ?? run?.stdout ?? ""}
          />
        ) : null
      }
      raw={
        expanded
          ? rawBlocks(toolCall.function.arguments, rawContent)
          : undefined
      }
    />
  );
}

function rawBlocks(args: string, rawContent: string): TranscriptRawBlock[] {
  const blocks: TranscriptRawBlock[] = [{ label: "Arguments", value: args }];
  if (rawContent) blocks.push({ label: "Raw result", value: rawContent });
  return blocks;
}

function runStatusKey({
  waiting,
  extra,
  run,
  receipt,
  receiptPending,
  result,
}: {
  waiting: boolean;
  extra: PythonRunExtra | undefined;
  run: { status: string; finalStatus?: string } | undefined;
  receipt: boolean;
  receiptPending: boolean;
  result: ToolResult | undefined;
}): string | undefined {
  if (waiting) return "waiting";
  if (extra) return extra.status;
  if (run?.status === "finished") return run.finalStatus ?? "completed";
  if (run) return run.status;
  if (receiptPending) return "background";
  if (receipt) return "result_not_available";
  if (result) return result.tool_failed ? "failed" : "completed";
  return undefined;
}

function isEmptyResult(preview: string) {
  const trimmed = preview.trim();
  return trimmed === "" || trimmed === "null" || trimmed === "None";
}

function StateLine({
  extra,
  waiting,
  receipt,
  receiptPending,
  busy,
  backgroundLive,
  announce,
}: {
  announce: boolean;
  extra: PythonRunExtra | undefined;
  waiting: boolean;
  receipt: boolean;
  receiptPending: boolean;
  busy: boolean;
  backgroundLive: boolean;
}) {
  if (waiting)
    return (
      <p className={styles.stateWarning} role="status">
        {WAITING_NOTE}
      </p>
    );
  if (!extra) {
    if (receiptPending || backgroundLive)
      return <p className={styles.stateMuted}>{BACKGROUND_NOTE}</p>;
    if (receipt && !busy)
      return (
        <p className={styles.stateMuted}>
          The background result is not available.
        </p>
      );
    return null;
  }
  if (extra.status === "cancelled") {
    const pending = extra.calls.some((call) => call.approval === "cancelled");
    return (
      <p className={styles.stateMuted}>
        {pending
          ? "Stopped while waiting for approval. The pending call did not run."
          : "Stopped before it finished."}
      </p>
    );
  }
  if (extra.status === "limit_exceeded")
    return (
      <p className={styles.stateWarning} role={announce ? "status" : undefined}>
        {limitSentence(extra)}
      </p>
    );
  if (extra.error)
    return (
      <p className={styles.stateDanger} role={announce ? "alert" : undefined}>
        <span className={styles.errorType}>{extra.error.type}</span>
        {extra.error.message && `: ${extra.error.message}`}
      </p>
    );
  if (extra.status === "timed_out")
    return (
      <p className={styles.stateDanger} role={announce ? "alert" : undefined}>
        Timed out before it finished.
      </p>
    );
  return null;
}

type BodyProps = {
  id: string;
  code: string;
  codeCut: boolean;
  announce: boolean;
  extra: PythonRunExtra | undefined;
  rawContent: string;
  liveStdout: string;
  calls: readonly PythonCallRecord[];
  waitingCallIds: ReadonlySet<string>;
  toolNames: ReadonlySet<string>;
  waiting: boolean;
  receipt: boolean;
  receiptPending: boolean;
  busy: boolean;
  backgroundLive: boolean;
  stopError: string | undefined;
  changes: ScriptChange[];
  artifactsDir: string | undefined;
  roots: WorkspaceRoots;
  expanded: boolean;
  onToggleExpanded: () => void;
  toolCallId: string;
  displays: React.ReactNode;
};

function ScriptBody({
  id,
  code,
  codeCut,
  announce,
  extra,
  rawContent,
  liveStdout,
  calls,
  waitingCallIds,
  toolNames,
  waiting,
  receipt,
  receiptPending,
  busy,
  backgroundLive,
  stopError,
  changes,
  artifactsDir,
  roots,
  expanded,
  onToggleExpanded,
  toolCallId,
  displays,
}: BodyProps) {
  const lineCount = useMemo(() => splitCodeLines(code).length, [code]);
  const unplaced = useMemo(
    () => placeCalls(calls, code.trim() ? lineCount : 0).unplaced,
    [calls, code, lineCount],
  );
  const unplacedModels = unplaced.filter(isModelCall).length;
  const unplacedLabel =
    unplacedModels === 0
      ? "Tool calls"
      : unplacedModels === unplaced.length
        ? "Model calls"
        : "Calls";
  const artifacts = useMemo(
    () => (extra ? scriptArtifacts(extra, artifactsDir) : []),
    [artifactsDir, extra],
  );
  const stopped = extra?.status === "cancelled";
  const result =
    extra && !isEmptyResult(extra.result_preview)
      ? extra.result_preview
      : undefined;
  const stdout = extra ? extra.stdout_preview : liveStdout;
  const fallbackText =
    !extra && !receipt && !waiting && !busy && rawContent ? rawContent : "";
  return (
    <EngineChangesContext.Provider value={extra?.engine_changes}>
      <div className={styles.body} id={id}>
        {stopError && (
          <p role="alert" className={styles.stateDanger}>
            {stopError}
          </p>
        )}
        <StateLine
          extra={extra}
          waiting={waiting}
          receipt={receipt}
          receiptPending={receiptPending}
          busy={busy}
          backgroundLive={backgroundLive}
          announce={announce}
        />
        {code.trim() ? (
          <ScriptCodeView
            code={code}
            calls={calls}
            waitingCallIds={waitingCallIds}
            toolNames={toolNames}
            errorLine={stopped ? undefined : extra?.error?.line}
            expanded={expanded}
            onToggleExpanded={onToggleExpanded}
            root={roots}
          />
        ) : null}
        {codeCut && (
          <p className={styles.stateMuted}>
            Code truncated · the arguments were cut before the end.
          </p>
        )}
        {extra?.calls_truncated && (
          <p className={styles.stateMuted}>
            Showing {extra.calls.length} of {extra.calls_total} calls
          </p>
        )}
        {unplaced.length > 0 && (
          <section className={styles.block} aria-label={unplacedLabel}>
            <span className={styles.label}>
              {unplacedLabel} · {unplaced.length}
            </span>
            <NestedToolCalls
              calls={unplaced}
              waitingCallIds={waitingCallIds}
              root={roots}
            />
          </section>
        )}
        {displays}
        {expanded && extra?.error?.traceback && !stopped && (
          <details className={styles.details}>
            <summary>Traceback</summary>
            <ClampedText text={extra.error.traceback} lines={TRACEBACK_LINES} />
          </details>
        )}
        {result !== undefined && (
          <OutcomePreview
            label="Result"
            text={result}
            totalBytes={extra?.result_bytes}
            cutByEngine={extra?.result_truncated === true}
            expanded={expanded}
            onShowMore={onToggleExpanded}
          />
        )}
        {stdout && (result === undefined || expanded) && (
          <OutcomePreview
            label="Output"
            text={stdout}
            totalBytes={extra?.stdout_bytes}
            liveTail={!extra && byteLength(stdout) >= LIVE_STDOUT_TAIL_NEAR}
            expanded={expanded}
            onShowMore={onToggleExpanded}
          />
        )}
        {stdout && result !== undefined && !expanded && (
          <button
            type="button"
            className={styles.hiddenOutput}
            aria-expanded={false}
            onClick={onToggleExpanded}
          >
            Printed output hidden ·{" "}
            {plural(stdout.replace(/\n+$/, "").split("\n").length, "line")}
          </button>
        )}
        {fallbackText && (
          <OutcomePreview
            label="Result"
            text={fallbackText}
            expanded={expanded}
            onShowMore={onToggleExpanded}
          />
        )}
        {extra?.engine_changes && (
          <EngineChangesSection
            changes={extra.engine_changes}
            total={extra.engine_changes_total}
          />
        )}
        {changes.length > 0 && (
          <ChangesList
            changes={changes}
            roots={roots}
            toolCallId={toolCallId}
          />
        )}
        {artifacts.length > 0 && (
          <ArtifactList
            artifacts={artifacts}
            roots={artifactsDir ? [artifactsDir] : roots}
          />
        )}
        {expanded && extra && <RunFacts extra={extra} />}
      </div>
    </EngineChangesContext.Provider>
  );
}

function OutcomePreview({
  label,
  text,
  totalBytes,
  liveTail = false,
  cutByEngine = false,
  expanded,
  onShowMore,
}: {
  label: string;
  text: string;
  totalBytes?: number;
  liveTail?: boolean;
  cutByEngine?: boolean;
  expanded: boolean;
  onShowMore: () => void;
}) {
  const [full, setFull] = useState(false);
  const { shown, cut, size } = useMemo(() => {
    const trimmed = text.replace(/\n+$/, "");
    const visible = trimmed.slice(
      0,
      codePointBoundary(trimmed, PREVIEW_MAX_CHARS),
    );
    const bytes = byteLength(text);
    return {
      shown: visible,
      cut:
        (totalBytes !== undefined && totalBytes > bytes) ||
        visible.length < trimmed.length,
      size: totalBytes ?? bytes,
    };
  }, [text, totalBytes]);
  const json = useMemo(
    () =>
      label === "Result" && text.length <= JSON_HIGHLIGHT_MAX_CHARS
        ? prettyJson(text)
        : undefined,
    [label, text],
  );
  const overflowRef = useRef<HTMLDivElement>(null);
  const [overflowing, setOverflowing] = useState(false);
  useEffect(() => {
    const node = overflowRef.current;
    if (!node) return;
    const measure = () =>
      setOverflowing(node.scrollHeight > node.clientHeight + 1);
    measure();
    const observer =
      typeof ResizeObserver === "undefined"
        ? undefined
        : new ResizeObserver(measure);
    observer?.observe(node);
    return () => observer?.disconnect();
  }, [expanded, full, shown]);
  const clipped = !expanded && overflowing;
  return (
    <section className={styles.block} aria-label={label}>
      <span className={styles.labelRow}>
        <span className={styles.label}>{label}</span>
        {liveTail ? (
          <span className={styles.labelNote}>
            showing the last {formatBytes(LIVE_STDOUT_TAIL_BYTES)}
          </span>
        ) : (
          cut && (
            <span className={styles.labelNote}>
              Preview truncated · full {label.toLowerCase()} {formatBytes(size)}
            </span>
          )
        )}
        {cutByEngine && (
          <span className={styles.labelNote}>{label} cut by the engine</span>
        )}
      </span>
      <div className={styles.previewFrame}>
        <div
          ref={overflowRef}
          className={classNames(
            styles.preview,
            json !== undefined && styles.previewJson,
            expanded
              ? full
                ? styles.previewFull
                : styles.previewTall
              : styles.previewCompact,
          )}
          data-clipped={clipped || undefined}
          tabIndex={expanded && overflowing ? 0 : undefined}
          data-testid="script-preview"
        >
          {json !== undefined ? (
            <ShikiCodeBlock className="language-json" wrap>
              {json}
            </ShikiCodeBlock>
          ) : (
            shown
          )}
        </div>
      </div>
      {clipped && (
        <button
          type="button"
          className={styles.linkButton}
          aria-expanded={false}
          onClick={onShowMore}
        >
          Show more
        </button>
      )}
      {expanded && (overflowing || full) && (
        <button
          type="button"
          className={styles.linkButton}
          aria-expanded={full}
          onClick={() => setFull((value) => !value)}
        >
          {full
            ? `Collapse ${label.toLowerCase()}`
            : `Expand ${label.toLowerCase()}`}
        </button>
      )}
    </section>
  );
}

/** Accessible file name: the basename, or the relative path when names repeat. */
function fileLabel(path: string, roots: WorkspaceRoots, repeated: boolean) {
  const { name, dir } = splitDisplayPath(path, roots);
  return repeated && dir ? `${dir}/${name}` : name;
}

function repeatedNames(paths: readonly string[], roots: WorkspaceRoots) {
  const seen = new Map<string, number>();
  for (const path of paths) {
    const { name } = splitDisplayPath(path, roots);
    seen.set(name, (seen.get(name) ?? 0) + 1);
  }
  return new Set(
    [...seen].filter(([, count]) => count > 1).map(([name]) => name),
  );
}

function PathText({ path, roots }: { path: string; roots: WorkspaceRoots }) {
  const { name, dir } = splitDisplayPath(path, roots);
  return (
    <span className={styles.path} title={path}>
      <span className={styles.pathName}>{name}</span>
      {dir && <span className={styles.pathDir}>{dir}</span>}
    </span>
  );
}

function ChangesList({
  changes,
  roots,
  toolCallId,
}: {
  changes: ScriptChange[];
  roots: WorkspaceRoots;
  toolCallId: string;
}) {
  const [all, setAll] = useState(false);
  const repeated = useMemo(
    () =>
      repeatedNames(
        changes.map((change) => change.path),
        roots,
      ),
    [changes, roots],
  );
  const shown = all ? changes : changes.slice(0, MAX_CHANGED_FILES);
  const hidden = changes.length - shown.length;
  return (
    <section className={styles.block} aria-label="Changes">
      <span className={styles.label}>Changes · {changes.length}</span>
      <ul className={styles.files}>
        {shown.map((change, index) => (
          <ChangeRow
            key={`${index}:${change.path}`}
            change={change}
            roots={roots}
            repeated={repeated}
            diffKey={`script:${toolCallId}:${index}`}
          />
        ))}
      </ul>
      {hidden > 0 && (
        <button
          type="button"
          className={styles.linkButton}
          onClick={() => setAll(true)}
        >
          Show {plural(hidden, "more file", "more files")}
        </button>
      )}
    </section>
  );
}

function ChangeRow({
  change,
  roots,
  repeated,
  diffKey,
}: {
  change: ScriptChange;
  roots: WorkspaceRoots;
  repeated: ReadonlySet<string>;
  diffKey: string;
}) {
  const { canOpen, openFile } = useOpenFileInApp();
  const store = useCollapsibleStore();
  const storeKey = `diff:${diffKey}`;
  if (store && store.get(storeKey) === undefined) store.set(storeKey, false);
  const diffs = useMemo(
    (): DiffMessage[] => [
      { role: "diff", tool_call_id: diffKey, content: change.chunks },
    ],
    [change.chunks, diffKey],
  );
  const { name } = splitDisplayPath(change.path, roots);
  const label = fileLabel(change.path, roots, repeated.has(name));
  if (change.chunks.length > 0)
    return (
      <li className={styles.diffScroll}>
        <GroupedDiffs diffs={diffs} />
      </li>
    );
  return (
    <li className={styles.fileRow}>
      <PathText path={change.path} roots={roots} />
      {change.bytes !== undefined && (
        <span className={styles.fileMeta}>{formatBytes(change.bytes)}</span>
      )}
      {canOpen && (
        <Button
          size="sm"
          variant="ghost"
          className={styles.fileAction}
          aria-label={`Open ${label}`}
          onClick={() => openFile({ path: change.path })}
        >
          Open
        </Button>
      )}
    </li>
  );
}

function ArtifactList({
  artifacts,
  roots,
}: {
  artifacts: { path: string; bytes: number }[];
  roots: WorkspaceRoots;
}) {
  const { canOpen, openFile } = useOpenFileInApp();
  const copy = useCopyToClipboard();
  const repeated = useMemo(
    () =>
      repeatedNames(
        artifacts.map((file) => file.path),
        roots,
      ),
    [artifacts, roots],
  );
  return (
    <section className={styles.block} aria-label="Artifacts">
      <span className={styles.label}>Artifacts · {artifacts.length}</span>
      <ul className={styles.files}>
        {artifacts.map((file, index) => {
          const { name } = splitDisplayPath(file.path, roots);
          const label = fileLabel(file.path, roots, repeated.has(name));
          return (
            <li key={`${index}:${file.path}`} className={styles.fileRow}>
              <PathText path={file.path} roots={roots} />
              <span className={styles.fileMeta}>{formatBytes(file.bytes)}</span>
              {canOpen && (
                <Button
                  size="sm"
                  variant="ghost"
                  className={styles.fileAction}
                  aria-label={`Open ${label}`}
                  onClick={() => openFile({ path: file.path })}
                >
                  Open
                </Button>
              )}
              <IconButton
                size="sm"
                variant="ghost"
                icon={Copy}
                className={styles.fileAction}
                aria-label={`Copy path of ${label}`}
                title="Copy path"
                onClick={() => copy(file.path)}
              />
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function RunFacts({ extra }: { extra: PythonRunExtra }) {
  const facts: TranscriptFact[] = [
    {
      label: "Duration",
      value: formatCallDuration(
        Math.max(0, extra.finished_at_ms - extra.started_at_ms),
      ),
    },
    {
      label: "Tool calls",
      value: String(
        callCounts(extra.calls, extra.calls_total, extra.llm?.calls).tools,
      ),
    },
    {
      label: "Session",
      value: sessionSummary(extra),
      title: sessionSummary(extra),
    },
    {
      label: "Run ID",
      value: <code className={styles.mono}>{extra.run_id}</code>,
      title: extra.run_id,
    },
  ];
  return (
    <>
      <TranscriptFacts facts={facts} />
      {extra.llm && extra.llm.calls > 0 && <ModelTotals llm={extra.llm} />}
    </>
  );
}

/** One totals line: "12 calls · 41.2k in · 3.1k out · gpt-5-mini, claude-haiku" */
function ModelTotals({ llm }: { llm: PythonLlmTotals }) {
  const value = [
    plural(llm.calls, "call"),
    `${compactCount(llm.input_tokens)} in`,
    `${compactCount(llm.output_tokens)} out`,
    ...(llm.models.length > 0 ? [llm.models.join(", ")] : []),
  ].join(" · ");
  return (
    <p
      className={styles.totals}
      title={`${formatCount(llm.input_tokens)} input tokens, ${formatCount(
        llm.output_tokens,
      )} output tokens`}
    >
      <span className={styles.label}>Model calls</span>
      <span>{value}</span>
    </p>
  );
}

function ScriptActions({
  code,
  codeCut,
  extra,
  stdout,
}: {
  code: string;
  codeCut: boolean;
  extra: PythonRunExtra | undefined;
  stdout: string;
}) {
  const copy = useCopyToClipboard();
  const dispatch = useAppDispatch();
  const { canOpen, openFile } = useOpenFileInApp();
  const runDir = extra?.run_dir;
  return (
    <>
      {code && (
        <Button
          leftIcon={Copy}
          size="sm"
          variant="soft"
          disabled={codeCut}
          title={codeCut ? "The code was cut before the end" : undefined}
          onClick={() => copy(code)}
        >
          Copy code
        </Button>
      )}
      {stdout && (
        <Button
          leftIcon={Copy}
          size="sm"
          variant="soft"
          onClick={() => copy(stdout)}
        >
          Copy output
        </Button>
      )}
      {canOpen && runDir && (
        <Button
          leftIcon={FileCode}
          size="sm"
          variant="soft"
          onClick={() => openFile({ path: `${runDir}/code.py` })}
        >
          Open code
        </Button>
      )}
      {canOpen && runDir && (
        <Button
          leftIcon={FileOutput}
          size="sm"
          variant="soft"
          onClick={() => openFile({ path: `${runDir}/stdout.txt` })}
        >
          Open full output
        </Button>
      )}
      <Button
        leftIcon={FlaskConical}
        size="sm"
        variant="soft"
        onClick={() => {
          dispatch(setDockSection("python"));
          dispatch(setDockOpen(true));
        }}
      >
        Show workspace
      </Button>
    </>
  );
}

const Elapsed = memo(function Elapsed({
  started,
  finished,
  busy,
}: {
  started: number;
  finished?: number | null;
  busy: boolean;
}) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!busy) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [busy]);
  return (
    <span className={styles.elapsed}>
      {formatCallDuration(Math.max(0, (finished ?? now) - started))}
    </span>
  );
});
