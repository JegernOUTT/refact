import { memo, useContext, useId, useMemo, useState } from "react";
import classNames from "classnames";
import { ChevronRight } from "lucide-react";
import type {
  DiffMessage,
  PythonCallRecord,
} from "../../services/refact/types";
import { GroupedDiffs } from "./DiffContent";
import { Icon } from "../ui";
import { ArgRows, ClampedText } from "./ToolCard/PythonBlocks";
import { visibleText } from "./ToolCard/safeText";
import {
  APPROVAL_LABEL,
  CALL_STATUS,
  CALL_STATUS_TONE,
  diffMessagesFor,
  formatCallDuration,
  orderedArgEntries,
  readPythonArgs,
  summarizePythonArgs,
  truncateText,
  type CallStatusKey,
  type WorkspaceRoots,
} from "./ToolCard/pythonDisplay";
import {
  callModel,
  callTokens,
  compactCount,
  isModelCall,
  readLlmPreview,
  tokensInOut,
} from "./ToolCard/scriptDisplays";
import {
  ENGINE_CLASS_LABEL,
  EngineChangesContext,
  engineCallClass,
  engineFunction,
  isEngineTool,
} from "./ToolCard/engineChanges";
import styles from "./ToolCard/PythonTool.module.css";

/** Long lists keep the newest calls visible; earlier ones expand on demand. */
const VISIBLE_CALLS = 20;

export function NestedToolCalls({
  calls,
  waitingCallIds,
  root,
  limit = VISIBLE_CALLS,
}: {
  calls: readonly PythonCallRecord[];
  waitingCallIds?: ReadonlySet<string>;
  root?: WorkspaceRoots;
  limit?: number;
}) {
  const rows = useMemo(
    () =>
      [...calls]
        .sort((a, b) => a.seq - b.seq)
        .map((record) => ({ record, diffs: diffMessagesFor(record) })),
    [calls],
  );
  const [showAll, setShowAll] = useState(false);
  if (rows.length === 0) return null;
  const hidden = showAll ? 0 : Math.max(0, rows.length - limit);
  return (
    <ol className={styles.calls}>
      {hidden > 0 && (
        <li className={styles.call}>
          <button
            type="button"
            className={styles.linkButton}
            onClick={() => setShowAll(true)}
          >
            Show {hidden} earlier {hidden === 1 ? "call" : "calls"}
          </button>
        </li>
      )}
      {rows.slice(hidden).map(({ record, diffs }) => (
        <NestedCallRow
          key={record.seq}
          record={record}
          diffs={diffs}
          waiting={waitingCallIds?.has(record.call_id) ?? false}
          root={root}
        />
      ))}
    </ol>
  );
}

const NestedCallRow = memo(function NestedCallRow({
  record,
  diffs,
  waiting,
  root,
}: {
  record: PythonCallRecord;
  diffs: DiffMessage[];
  waiting: boolean;
  root?: WorkspaceRoots;
}) {
  const [open, setOpen] = useState(false);
  const bodyId = useId();
  const status: CallStatusKey =
    waiting && record.status === "running" ? "waiting" : record.status;
  const view = CALL_STATUS[status];
  const model = isModelCall(record);
  const engine = isEngineTool(record.tool);
  const summary = useMemo(
    () =>
      model
        ? modelCallSummary(record)
        : engine
          ? engineCallSummary(record)
          : summarizePythonArgs(record.args_preview, root),
    [engine, model, record, root],
  );
  const tokens = model ? callTokens(record) : undefined;
  const args = useMemo(
    () => (open && !model ? readPythonArgs(record.args_preview) : null),
    [model, open, record.args_preview],
  );
  const approval = APPROVAL_LABEL[record.approval];
  return (
    <li className={styles.call} data-status={status}>
      <button
        type="button"
        className={styles.callRow}
        aria-expanded={open}
        aria-controls={bodyId}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon
          icon={ChevronRight}
          size="sm"
          tone="muted"
          className={classNames(styles.chevron, open && styles.chevronOpen)}
        />
        <span className={styles.callIcon} title={view.label}>
          <Icon
            icon={view.icon}
            size="sm"
            tone={CALL_STATUS_TONE[status]}
            className={view.className}
          />
        </span>
        {status !== "waiting" && (
          <span className={styles.srOnly}>{view.label}</span>
        )}
        <span className={styles.callName}>{record.tool}</span>
        <span className={styles.callSummary} title={summary || undefined}>
          {summary}
        </span>
        {status === "waiting" && (
          <span className={styles.callWaiting}>Waiting for your approval</span>
        )}
        {approval && <span className={styles.callMeta}>{approval}</span>}
        {tokens !== undefined && (
          <span className={styles.callMeta}>{compactCount(tokens)} tokens</span>
        )}
        {record.duration_ms !== null && (
          <span className={styles.callMeta}>
            {formatCallDuration(record.duration_ms)}
          </span>
        )}
      </button>
      {open && (
        <div id={bodyId} className={styles.callBody}>
          <CallDetails record={record} diffs={diffs} args={args} root={root} />
        </div>
      )}
    </li>
  );
});

const DETAIL_LINES = 8;

/** "gpt-5-mini · Summarise the failing tests in one line…" */
function modelCallSummary(record: PythonCallRecord): string {
  const model = callModel(record);
  const prompt = readLlmPreview(record.args_preview)
    .prompt?.replace(/\s+/g, " ")
    .trim();
  return truncateText([model, prompt].filter(Boolean).join(" · "), 200);
}

/** "label: old → new" or a read's short result, else the arguments. */
function engineCallSummary(record: PythonCallRecord): string {
  const text =
    record.summary.trim() || summarizePythonArgs(record.args_preview);
  return truncateText(visibleText(text.replace(/\s+/g, " ")), 200);
}

/** Function, class in words, result summary and arguments of an engine call. */
function EngineCallDetails({
  record,
  args,
}: {
  record: PythonCallRecord;
  args: ReturnType<typeof readPythonArgs>;
}) {
  const changes = useContext(EngineChangesContext);
  const kind = engineCallClass(record, changes);
  const parsed = useMemo(
    () => args ?? readPythonArgs(record.args_preview),
    [args, record.args_preview],
  );
  const facts: [string, string][] = [["Function", engineFunction(record.tool)]];
  if (kind) facts.push(["Kind", ENGINE_CLASS_LABEL[kind]]);
  if (record.duration_ms !== null)
    facts.push(["Duration", formatCallDuration(record.duration_ms)]);
  facts.push(["Status", CALL_STATUS[record.status].label]);
  const failed = record.status === "failed" || record.status === "denied";
  return (
    <>
      <dl className={styles.args}>
        {facts.map(([key, value]) => (
          <div key={key} className={styles.arg}>
            <dt className={styles.argKey}>{key}</dt>
            <dd className={classNames(styles.argValue, styles.argInline)}>
              {value}
            </dd>
          </div>
        ))}
      </dl>
      {record.summary && (
        <div className={styles.callResult}>
          <span
            className={classNames(
              styles.sectionLabel,
              failed && styles.dangerLabel,
            )}
          >
            {failed ? "Error" : "Result"}
          </span>
          <ClampedText
            text={visibleText(record.summary)}
            lines={DETAIL_LINES}
          />
        </div>
      )}
      {parsed && Object.keys(parsed).length > 0 && (
        <div className={styles.callResult}>
          <span className={styles.sectionLabel}>Arguments</span>
          <ArgRows entries={orderedArgEntries(parsed)} lines={DETAIL_LINES} />
        </div>
      )}
    </>
  );
}

/** Model, tokens, duration, status, the prompt and the answer or error of a model call. */
function ModelCallDetails({ record }: { record: PythonCallRecord }) {
  const preview = useMemo(
    () => readLlmPreview(record.args_preview),
    [record.args_preview],
  );
  const model = callModel(record);
  const tokens = tokensInOut(record);
  const facts: [string, string][] = [];
  if (model) facts.push(["Model", model]);
  if (tokens) facts.push(["Tokens", tokens]);
  if (record.duration_ms !== null)
    facts.push(["Duration", formatCallDuration(record.duration_ms)]);
  facts.push(["Status", CALL_STATUS[record.status].label]);
  if (preview.schema) facts.push(["Output", "Structured (schema)"]);
  const failed = record.status === "failed" || record.status === "denied";
  return (
    <>
      <dl className={styles.args}>
        {facts.map(([key, value]) => (
          <div key={key} className={styles.arg}>
            <dt className={styles.argKey}>{key}</dt>
            <dd className={classNames(styles.argValue, styles.argInline)}>
              {value}
            </dd>
          </div>
        ))}
      </dl>
      {preview.prompt && (
        <div className={styles.callResult}>
          <span className={styles.sectionLabel}>Prompt</span>
          <ClampedText text={preview.prompt} lines={DETAIL_LINES} />
        </div>
      )}
      {record.summary && (
        <div className={styles.callResult}>
          <span
            className={classNames(
              styles.sectionLabel,
              failed && styles.dangerLabel,
            )}
          >
            {failed ? "Error" : "Answer"}
          </span>
          <ClampedText text={record.summary} lines={DETAIL_LINES} />
        </div>
      )}
    </>
  );
}

/** Arguments, result summary and diffs of one call. */
export function CallDetails({
  record,
  diffs,
  args,
  root,
}: {
  record: PythonCallRecord;
  diffs: DiffMessage[];
  args: ReturnType<typeof readPythonArgs>;
  root?: WorkspaceRoots;
}) {
  if (isModelCall(record)) return <ModelCallDetails record={record} />;
  if (isEngineTool(record.tool))
    return <EngineCallDetails record={record} args={args} />;
  return (
    <>
      {args ? (
        <ArgRows
          entries={orderedArgEntries(args)}
          root={root}
          lines={DETAIL_LINES}
        />
      ) : (
        record.args_preview.trim() && (
          <ClampedText text={record.args_preview} lines={DETAIL_LINES} />
        )
      )}
      {record.summary && (
        <div className={styles.callResult}>
          <span className={styles.sectionLabel}>Result</span>
          <ClampedText text={record.summary} lines={DETAIL_LINES} />
        </div>
      )}
      {diffs[0].content.length > 0 && (
        <div className={styles.diffScroll}>
          <GroupedDiffs diffs={diffs} />
        </div>
      )}
    </>
  );
}
