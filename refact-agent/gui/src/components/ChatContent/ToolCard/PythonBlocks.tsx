import {
  memo,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import classNames from "classnames";
import { ShikiCodeBlock } from "../../Markdown";
import { Icon } from "../../ui";
import type { PythonCallRecord } from "../../../services/refact/types";
import { CallDetails, NestedToolCalls } from "../NestedToolCalls";
import {
  APPROVAL_LABEL,
  CALL_STATUS,
  CALL_STATUS_TONE,
  diffMessagesFor,
  useCodeTokens,
  type CodeToken,
  aggregateCallStatus,
  callDiffStats,
  calleeAtColumn,
  callsDuration,
  clampLines,
  cutCodeLine,
  foldAnchors,
  foldCodeRows,
  formatCallDuration,
  lexicalCallees,
  placeCalls,
  plural,
  readPythonArgs,
  splitCodeLines,
  type CallStatusKey,
  type CodeRow,
  type CalleeRange,
  formatArgValue,
  isPathKey,
  relativeToRoot,
  type WorkspaceRoots,
} from "./pythonDisplay";
import { callsTokens } from "./scriptDisplayHelpers";
import styles from "./PythonTool.module.css";

/**
 * Monospace text block that shows the first `lines` lines with a
 * "Show more" control. `json` highlights through the shared code block;
 * everything else is a plain `<pre>` so text is never parsed as Markdown.
 */
export function ClampedText({
  text,
  lines,
  json = false,
  label,
}: {
  text: string;
  lines: number;
  json?: boolean;
  label?: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const clamp = useMemo(() => clampLines(text, lines), [text, lines]);
  const shown = expanded || !clamp.clipped ? clamp.full : clamp.head;
  return (
    <div className={styles.clamped}>
      {json ? (
        <div className={styles.codeBlock}>
          <ShikiCodeBlock
            className="language-json"
            showLineNumbers={false}
            wrap
          >
            {shown}
          </ShikiCodeBlock>
        </div>
      ) : (
        <pre className={styles.pre} aria-label={label}>
          {shown}
        </pre>
      )}
      {clamp.clipped && (
        <button
          type="button"
          className={styles.linkButton}
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded
            ? "Show less"
            : `Show ${clamp.total - lines} more ${
                clamp.total - lines === 1 ? "line" : "lines"
              }`}
        </button>
      )}
    </div>
  );
}

/**
 * Full text in a scrollable block of bounded height; nothing is hidden behind
 * a click. Used for what an approval would run. When the text is taller than
 * the block, a muted line says how many lines there are in total.
 */
export function ScrollText({
  text,
  label,
  visibleLines = 12,
}: {
  text: string;
  label: string;
  visibleLines?: number;
}) {
  const total = useMemo(
    () => clampLines(text, visibleLines),
    [text, visibleLines],
  );
  return (
    <div className={styles.clamped}>
      <pre
        className={classNames(styles.pre, styles.scrollPre)}
        aria-label={label}
        tabIndex={total.clipped ? 0 : undefined}
      >
        {text}
      </pre>
      {total.clipped && (
        <span className={styles.scrollNote}>
          {total.total} lines · scroll to see all
        </span>
      )}
    </div>
  );
}

/** Key and value rows for nested call or approval arguments. */
export function ArgRows({
  entries,
  root,
  lines,
}: {
  entries: [string, unknown][];
  root?: WorkspaceRoots;
  lines: number;
}) {
  if (entries.length === 0) return null;
  return (
    <dl className={styles.args}>
      {entries.map(([key, raw]) => {
        const formatted = formatArgValue(raw);
        const value =
          typeof raw === "string" && isPathKey(key)
            ? relativeToRoot(raw, root)
            : formatted;
        const multiline = value.includes("\n");
        return (
          <div key={key} className={styles.arg}>
            <dt className={styles.argKey}>{key}</dt>
            <dd
              className={classNames(
                styles.argValue,
                !multiline && styles.argInline,
              )}
              title={value !== formatted ? formatted : undefined}
            >
              {multiline ? <ClampedText text={value} lines={lines} /> : value}
            </dd>
          </div>
        );
      })}
    </dl>
  );
}

/** One code line as React text runs, with callee names emphasised. */
export function CodeLineText({
  text,
  tokens,
  callees,
}: {
  text: string;
  tokens: CodeToken[] | undefined;
  callees: readonly CalleeRange[];
}) {
  const runs = tokens ?? [{ text }];
  const parts: React.ReactNode[] = [];
  let offset = 0;
  for (const [tokenIndex, token] of runs.entries()) {
    const start = offset;
    if (start >= text.length) break;
    const end = Math.min(start + token.text.length, text.length);
    offset = start + token.text.length;
    const cuts = [start, end];
    for (const range of callees) {
      if (range.start > start && range.start < end) cuts.push(range.start);
      if (range.end > start && range.end < end) cuts.push(range.end);
    }
    cuts.sort((a, b) => a - b);
    for (let i = 0; i + 1 < cuts.length; i++) {
      const from = cuts[i];
      const to = cuts[i + 1];
      if (from === to) continue;
      const piece = text.slice(from, to);
      const callee = callees.some(
        (range) => from >= range.start && to <= range.end,
      );
      parts.push(
        callee ? (
          <span key={`${tokenIndex}:${from}`} className={styles.callee}>
            {piece}
          </span>
        ) : token.color ? (
          // Shiki theme colours are data, the same values its HTML carries inline.
          <span key={`${tokenIndex}:${from}`} style={{ color: token.color }}>
            {piece}
          </span>
        ) : (
          piece
        ),
      );
    }
  }
  return <>{parts}</>;
}

const MAX_DETAIL_CALLS = 20;
/** Expanded code renders this many lines at a time. */
export const CODE_EXPANDED_STEP = 400;
const NO_CALLEES: readonly CalleeRange[] = [];

type LineCalls = {
  calls: PythonCallRecord[];
  status: CallStatusKey;
};

function sameItems<T>(a: readonly T[], b: readonly T[]) {
  return a.length === b.length && a.every((item, index) => item === b[index]);
}

function sameRanges(a: readonly CalleeRange[], b: readonly CalleeRange[]) {
  return (
    a.length === b.length &&
    a.every(
      (range, index) =>
        range.start === b[index].start && range.end === b[index].end,
    )
  );
}

/** Keeps the previous value of each line when its content did not change. */
function useStableByLine<T>(
  next: ReadonlyMap<number, T>,
  same: (a: T, b: T) => boolean,
): ReadonlyMap<number, T> {
  const previous = useRef<ReadonlyMap<number, T>>(new Map());
  return useMemo(() => {
    const stable = new Map<number, T>();
    next.forEach((value, line) => {
      const old = previous.current.get(line);
      stable.set(line, old !== undefined && same(old, value) ? old : value);
    });
    previous.current = stable;
    return stable;
  }, [next, same]);
}

const sameLineCalls = (a: LineCalls, b: LineCalls) =>
  a.status === b.status && sameItems(a.calls, b.calls);

/**
 * The script's code as the run's progress view: tool calls are emphasised,
 * lines that made calls carry their status and open their details.
 */
export function ScriptCodeView({
  code,
  calls,
  waitingCallIds,
  toolNames,
  errorLine,
  expanded,
  onToggleExpanded,
  root,
}: {
  code: string;
  calls: readonly PythonCallRecord[];
  waitingCallIds: ReadonlySet<string>;
  toolNames: ReadonlySet<string>;
  errorLine?: number;
  expanded: boolean;
  onToggleExpanded: () => void;
  root?: WorkspaceRoots;
}) {
  const lines = useMemo(() => splitCodeLines(code), [code]);
  const tokens = useCodeTokens(code, lines);
  const { byLine } = useMemo(
    () => placeCalls(calls, lines.length),
    [calls, lines.length],
  );
  const freshLineCalls = useMemo(() => {
    const map = new Map<number, LineCalls>();
    byLine.forEach((list, line) =>
      map.set(line, {
        calls: list,
        status: aggregateCallStatus(list, waitingCallIds),
      }),
    );
    return map;
  }, [byLine, waitingCallIds]);
  const lineCalls = useStableByLine(freshLineCalls, sameLineCalls);
  const freshCallees = useMemo(() => {
    const map = new Map<number, readonly CalleeRange[]>();
    lines.forEach((text, index) => {
      const ranges = lexicalCallees(text, toolNames);
      for (const call of byLine.get(index + 1) ?? []) {
        if (call.col === undefined) continue;
        const range = calleeAtColumn(text, call.col);
        if (
          range &&
          !ranges.some(
            (other) => other.start === range.start && other.end === range.end,
          )
        )
          ranges.push(range);
      }
      if (ranges.length > 0) map.set(index + 1, ranges);
    });
    return map;
  }, [byLine, lines, toolNames]);
  const callees = useStableByLine(freshCallees, sameRanges);
  const compactRows = useMemo((): CodeRow[] => {
    const lineStatus = new Map<number, CallStatusKey>();
    lineCalls.forEach((made, line) => lineStatus.set(line, made.status));
    return foldCodeRows(
      lines,
      foldAnchors({ errorLine, lineStatus, callLines: callees.keys() }),
    );
  }, [callees, errorLine, lineCalls, lines]);
  const [shownLines, setShownLines] = useState(CODE_EXPANDED_STEP);
  const rows = useMemo(
    (): CodeRow[] =>
      expanded
        ? lines
            .slice(0, shownLines)
            .map((_, index) => ({ kind: "line", number: index + 1 }))
        : compactRows,
    [compactRows, expanded, lines, shownLines],
  );
  const foldable = compactRows.some((row) => row.kind === "fold");
  const [openLines, setOpenLines] = useState<ReadonlySet<number>>(
    () => new Set(),
  );
  const toggleLine = useCallback(
    (line: number) =>
      setOpenLines((current) => {
        const next = new Set(current);
        if (next.has(line)) next.delete(line);
        else next.add(line);
        return next;
      }),
    [],
  );
  const listRef = useRef<HTMLDivElement>(null);
  const focusLine = useRef<number>();
  const openFold = (from: number) => {
    focusLine.current = from;
    if (from > shownLines)
      setShownLines(Math.ceil(from / CODE_EXPANDED_STEP) * CODE_EXPANDED_STEP);
    onToggleExpanded();
  };
  useEffect(() => {
    const line = focusLine.current;
    if (!expanded || line === undefined) return;
    focusLine.current = undefined;
    listRef.current
      ?.querySelector<HTMLElement>(`[data-line="${line}"]`)
      ?.focus();
  }, [expanded, rows]);
  const hiddenLines = expanded ? lines.length - rows.length : 0;
  return (
    <div
      className={classNames(styles.codeView, expanded && styles.codeExpanded)}
      data-testid="script-code"
      data-calls={lineCalls.size > 0}
      data-digits={Math.min(String(lines.length).length, MAX_GUTTER_DIGITS)}
    >
      <div
        className={styles.codeRows}
        role="list"
        aria-label="Code"
        ref={listRef}
      >
        {rows.map((row) =>
          row.kind === "fold" ? (
            <div key={`fold-${row.from}`} role="listitem">
              <button
                type="button"
                className={styles.foldRow}
                data-testid="script-code-fold"
                aria-label={`Show ${plural(
                  row.to - row.from + 1,
                  "hidden line",
                )}`}
                onClick={() => openFold(row.from)}
              >
                ⋯ {plural(row.to - row.from + 1, "line")}
              </button>
            </div>
          ) : (
            <CodeLineRow
              key={row.number}
              number={row.number}
              text={lines[row.number - 1]}
              tokens={tokens?.[row.number - 1]}
              callees={callees.get(row.number) ?? NO_CALLEES}
              made={lineCalls.get(row.number)}
              error={row.number === errorLine}
              open={openLines.has(row.number)}
              onToggle={toggleLine}
              waitingCallIds={
                lineCalls.has(row.number) ? waitingCallIds : undefined
              }
              root={root}
            />
          ),
        )}
      </div>
      {hiddenLines > 0 && (
        <button
          type="button"
          className={styles.codeControl}
          onClick={() => setShownLines((count) => count + CODE_EXPANDED_STEP)}
        >
          Show {plural(Math.min(hiddenLines, CODE_EXPANDED_STEP), "more line")}{" "}
          of {hiddenLines}
        </button>
      )}
      {foldable && (
        <button
          type="button"
          className={styles.codeControl}
          aria-expanded={expanded}
          onClick={onToggleExpanded}
        >
          {expanded
            ? "Show less"
            : `Show all · ${plural(lines.length, "line")}`}
        </button>
      )}
    </div>
  );
}

const MAX_GUTTER_DIGITS = 5;

function lineAnnotation(made: LineCalls): string {
  const parts: string[] = [];
  if (made.calls.length > 1) parts.push(`×${made.calls.length}`);
  const approvals = new Set(
    made.calls.map((call) => APPROVAL_LABEL[call.approval]).filter(Boolean),
  );
  if (approvals.size === 1) parts.push([...approvals][0]);
  const stats = callDiffStats(made.calls);
  if (stats) parts.push(stats);
  const tokens = callsTokens(made.calls);
  if (tokens) parts.push(tokens);
  const duration = callsDuration(made.calls);
  if (duration !== undefined) parts.push(formatCallDuration(duration));
  return parts.join(" · ");
}

function SingleCallDetails({
  record,
  root,
}: {
  record: PythonCallRecord;
  root?: WorkspaceRoots;
}) {
  const args = useMemo(
    () => readPythonArgs(record.args_preview),
    [record.args_preview],
  );
  return (
    <CallDetails
      record={record}
      diffs={diffMessagesFor(record)}
      args={args}
      root={root}
    />
  );
}

const CodeLineRow = memo(function CodeLineRow({
  number,
  text,
  tokens,
  callees,
  made,
  error,
  open,
  onToggle,
  waitingCallIds,
  root,
}: {
  number: number;
  text: string;
  tokens: CodeToken[] | undefined;
  callees: readonly CalleeRange[];
  made: LineCalls | undefined;
  error: boolean;
  open: boolean;
  onToggle: (line: number) => void;
  waitingCallIds: ReadonlySet<string> | undefined;
  root?: WorkspaceRoots;
}) {
  const detailId = useId();
  const statusId = useId();
  const { shown, hidden } = cutCodeLine(text);
  const status = [
    made ? CALL_STATUS[made.status].label : "",
    error ? "Failed here" : "",
  ]
    .filter(Boolean)
    .join(", ");
  const content = (
    <>
      <span className={styles.lineNumber} aria-hidden>
        {number}
      </span>
      <span className={styles.lineStatus}>
        {made && (
          <Icon
            icon={CALL_STATUS[made.status].icon}
            size="sm"
            tone={CALL_STATUS_TONE[made.status]}
            className={CALL_STATUS[made.status].className}
          />
        )}
      </span>
      <code className={styles.lineCode}>
        <CodeLineText text={shown} tokens={tokens} callees={callees} />
        {hidden > 0 && (
          <span className={styles.lineCut}>
            {" "}
            … {plural(hidden, "more character")}
          </span>
        )}
      </code>
      {made && (
        <span className={styles.lineNote}>
          {made.status === "waiting"
            ? "Waiting for your approval"
            : lineAnnotation(made)}
        </span>
      )}
    </>
  );
  const statusText = status && (
    <span id={statusId} className={styles.srOnly}>
      {status}
    </span>
  );
  if (!made)
    return (
      <div
        role="listitem"
        className={styles.codeLine}
        data-error={error || undefined}
        data-line={number}
        tabIndex={-1}
      >
        {content}
        {statusText}
      </div>
    );
  return (
    <div role="listitem" className={styles.callLineItem}>
      <button
        type="button"
        className={classNames(styles.codeLine, styles.callLine)}
        data-status={made.status}
        data-error={error || undefined}
        data-line={number}
        aria-expanded={open}
        aria-controls={open ? detailId : undefined}
        aria-describedby={statusId}
        onClick={() => onToggle(number)}
      >
        {content}
      </button>
      {statusText}
      {open && (
        <div id={detailId} className={styles.lineDetail}>
          {made.calls.length === 1 ? (
            <SingleCallDetails record={made.calls[0]} root={root} />
          ) : (
            <NestedToolCalls
              calls={made.calls}
              waitingCallIds={waitingCallIds}
              root={root}
              limit={MAX_DETAIL_CALLS}
            />
          )}
        </div>
      )}
    </div>
  );
});
