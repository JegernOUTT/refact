import {
  Component,
  Suspense,
  lazy,
  memo,
  useCallback,
  useId,
  useMemo,
  type ReactNode,
} from "react";
import classNames from "classnames";
import { ChartColumn, Copy, FileJson, Table2 } from "lucide-react";
import { Icon, IconButton, Tooltip } from "../../ui";
import { Markdown } from "../../Markdown";
import { TranscriptClamp } from "../TranscriptItem";
import { useCopyToClipboard } from "../../../hooks/useCopyToClipboard";
import { useOpenFileInApp } from "../../../hooks/useOpenFileInApp";
import type {
  PythonDisplay,
  PythonDisplayCell,
} from "../../../services/refact/types";
import {
  cellText,
  chartData,
  chartLabel,
  displayCopyText,
  displayKindView,
  displaySize,
  displayTitle,
  isChartKind,
  isDisplayPathInside,
  numericColumns,
  rowsNote,
  type ChartData,
} from "./scriptDisplayHelpers";
import { visibleText } from "./safeText";
import { useRememberedState } from "./useRememberedState";
import styles from "./ScriptDisplays.module.css";

/** A new lazy component per attempt, so a failed chunk load can be retried. */
function loadChart() {
  return lazy(() => import("./ScriptChart"));
}
let ScriptChart = loadChart();

/** Displays shown before "Show N more". */
export const DISPLAYS_SHOWN = 3;
/** Rows mounted at first and per "Show more rows" step. */
export const TABLE_ROWS_STEP = 50;
/** A cell longer than this is cut in the DOM; the full value opens on demand. */
export const CELL_PREVIEW_CHARS = 80;
/** Even an opened cell keeps at most this much text in the DOM. */
const CELL_FULL_MAX_CHARS = 5_000;
const EMPTY_OPENED: ReadonlyMap<number, ReadonlySet<number>> = new Map();

export type DisplayDensity = "card" | "rail";

/** Chart fallback: anything thrown while loading or drawing the chart. */
class ChartBoundary extends Component<
  { fallback: (retry: () => void) => ReactNode; children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  retry = () => {
    ScriptChart = loadChart();
    this.setState({ failed: false });
  };
  render() {
    return this.state.failed
      ? this.props.fallback(this.retry)
      : this.props.children;
  }
}

function cutText(text: string, max: number) {
  const chars = Array.from(text);
  return chars.length > max
    ? { text: `${chars.slice(0, max - 1).join("")}…`, cut: true }
    : { text, cut: false };
}

function CellContent({
  cell,
  open,
  rowIndex,
  columnIndex,
}: {
  cell: PythonDisplayCell | undefined;
  open: boolean;
  rowIndex: number;
  columnIndex: number;
}) {
  if (cell === null || cell === undefined)
    return (
      <>
        <span className={styles.nullCell} aria-hidden>
          —
        </span>
        <span className={styles.srOnly}>empty</span>
      </>
    );
  const text = visibleText(cellText(cell));
  const preview = cutText(text, CELL_PREVIEW_CHARS);
  if (!preview.cut) return <>{text}</>;
  const full = cutText(text, CELL_FULL_MAX_CHARS).text;
  return (
    <button
      type="button"
      className={classNames(styles.cellButton, open && styles.cellOpen)}
      aria-expanded={open}
      data-cell={`${rowIndex}:${columnIndex}`}
      title={open ? undefined : cutText(text, 500).text}
    >
      {open ? full : preview.text}
    </button>
  );
}

const TableRow = memo(function TableRow({
  row,
  rowIndex,
  width,
  numeric,
  openColumns,
}: {
  row: PythonDisplayCell[];
  rowIndex: number;
  width: number;
  numeric: readonly boolean[];
  openColumns: ReadonlySet<number> | undefined;
}) {
  return (
    <tr>
      {Array.from({ length: width }, (_, index) => (
        <td
          key={index}
          className={classNames(numeric[index] && styles.numeric)}
        >
          <CellContent
            cell={row[index]}
            open={openColumns?.has(index) ?? false}
            rowIndex={rowIndex}
            columnIndex={index}
          />
        </td>
      ))}
    </tr>
  );
});

export function DisplayTable({
  display,
  label,
  density = "card",
  stateKey,
}: {
  display: PythonDisplay;
  label: string;
  density?: DisplayDensity;
  /** Keeps shown rows and opened cells across remounts by the virtual list. */
  stateKey?: string;
}) {
  const columns = useMemo(() => display.columns ?? [], [display.columns]);
  const rows = useMemo(() => display.rows ?? [], [display.rows]);
  const width = useMemo(
    () => Math.max(columns.length, ...rows.map((row) => row.length), 0),
    [columns, rows],
  );
  const numeric = useMemo(() => numericColumns(columns, rows), [columns, rows]);
  const [limit, setLimit] = useRememberedState(
    stateKey && `${stateKey}:rows`,
    TABLE_ROWS_STEP,
  );
  // Opened cells, one map for the table: row index -> column indexes.
  const [opened, setOpened] = useRememberedState<
    ReadonlyMap<number, ReadonlySet<number>>
  >(stateKey && `${stateKey}:cells`, EMPTY_OPENED);
  const toggleCell = useCallback(
    (event: React.MouseEvent<HTMLElement>) => {
      const target = (event.target as HTMLElement).closest<HTMLElement>(
        "[data-cell]",
      );
      const key = target?.dataset.cell;
      if (!key) return;
      const [row, column] = key.split(":").map(Number);
      setOpened((current) => {
        const next = new Map(current);
        const columnsOpen = new Set(next.get(row));
        if (columnsOpen.has(column)) columnsOpen.delete(column);
        else columnsOpen.add(column);
        if (columnsOpen.size > 0) next.set(row, columnsOpen);
        else next.delete(row);
        return next;
      });
    },
    [setOpened],
  );
  const note = rowsNote(display);
  if (width === 0)
    return <p className={styles.note}>This table has no data.</p>;
  const shown = rows.slice(0, limit);
  const hidden = rows.length - shown.length;
  return (
    <>
      <div
        className={classNames(
          styles.tableScroll,
          density === "rail" && styles.tableScrollRail,
        )}
        role="region"
        aria-label={label}
        tabIndex={0}
        onClick={toggleCell}
      >
        <table className={styles.table}>
          <thead>
            <tr>
              {Array.from({ length: width }, (_, index) => {
                const name = columns.at(index);
                return (
                  <th
                    key={index}
                    scope="col"
                    className={classNames(numeric[index] && styles.numeric)}
                  >
                    {name !== undefined ? (
                      visibleText(name)
                    ) : (
                      <span className={styles.srOnly}>Column {index + 1}</span>
                    )}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {shown.map((row, rowIndex) => (
              <TableRow
                key={rowIndex}
                row={row}
                rowIndex={rowIndex}
                width={width}
                numeric={numeric}
                openColumns={opened.get(rowIndex)}
              />
            ))}
          </tbody>
        </table>
      </div>
      {hidden > 0 && (
        <button
          type="button"
          className={styles.linkButton}
          onClick={() => setLimit((value) => value + TABLE_ROWS_STEP)}
        >
          Show {Math.min(hidden, TABLE_ROWS_STEP)} more rows of {hidden}
        </button>
      )}
      {note && <p className={styles.note}>{note}</p>}
    </>
  );
}

function DisplayChart({
  display,
  data,
  asTable,
  density,
  stateKey,
}: {
  display: PythonDisplay;
  data: ChartData;
  asTable: boolean;
  density: DisplayDensity;
  stateKey?: string;
}) {
  const table = (
    <DisplayTable
      display={display}
      label={`${displayTitle(display)} data`}
      density={density}
      stateKey={stateKey}
    />
  );
  if (asTable) return table;
  const compact = density === "rail";
  const placeholder = (
    <div
      className={styles.chartCanvas}
      data-testid="script-chart-placeholder"
    />
  );
  return (
    <>
      <ChartBoundary
        fallback={(retry) => (
          <>
            {table}
            <button type="button" className={styles.linkButton} onClick={retry}>
              The chart could not be drawn · Try again
            </button>
          </>
        )}
      >
        <div
          className={classNames(
            styles.chartFrame,
            compact && styles.chartFrameRail,
          )}
          role="img"
          aria-label={chartLabel(display, data)}
        >
          <Suspense fallback={placeholder}>
            <ScriptChart data={data} />
          </Suspense>
        </div>
      </ChartBoundary>
      {rowsNote(display) && <p className={styles.note}>{rowsNote(display)}</p>}
    </>
  );
}

function DisplayMarkdown({
  text,
  density,
}: {
  text: string;
  density: DisplayDensity;
}) {
  return (
    <div
      className={classNames(
        styles.markdownFrame,
        density === "rail" && styles.markdownRail,
      )}
    >
      <TranscriptClamp lineCount={text.split("\n").length}>
        <Markdown inert>{visibleText(text)}</Markdown>
      </TranscriptClamp>
    </div>
  );
}

/** The body of one display: table, chart, markdown, or nothing for other kinds. */
export function DisplayBody({
  display,
  data,
  asTable = false,
  density = "card",
  stateKey,
}: {
  display: PythonDisplay;
  /** Chart data computed once by the caller. */
  data: ChartData | undefined;
  asTable?: boolean;
  density?: DisplayDensity;
  stateKey?: string;
}) {
  if (display.kind === "markdown")
    return <DisplayMarkdown text={display.text ?? ""} density={density} />;
  if (display.kind === "table")
    return (
      <DisplayTable
        display={display}
        label={`${displayTitle(display)} table`}
        density={density}
        stateKey={stateKey}
      />
    );
  if (isChartKind(display.kind)) {
    if (!data)
      return (
        <DisplayTable
          display={display}
          label={`${displayTitle(display)} data`}
          density={density}
          stateKey={stateKey}
        />
      );
    return (
      <DisplayChart
        display={display}
        data={data}
        asTable={asTable}
        density={density}
        stateKey={stateKey}
      />
    );
  }
  return null;
}

/** "Open full data", offered only for a file inside the folder the run reported. */
export function OpenFullDataButton({
  display,
  title,
  baseDir,
}: {
  display: PythonDisplay;
  title: string;
  baseDir: string | undefined;
}) {
  const { canOpen, openFile } = useOpenFileInApp();
  if (!canOpen || !display.path) return null;
  const inside = isDisplayPathInside(display.path, baseDir);
  return (
    <Tooltip
      content={
        inside
          ? "Open full data"
          : "The data file is outside the script workspace"
      }
    >
      <IconButton
        icon={FileJson}
        size="sm"
        variant="ghost"
        aria-label={`Open full data of ${title}`}
        disabled={!inside}
        onClick={() => inside && openFile({ path: display.path })}
      />
    </Tooltip>
  );
}

/** One display: caption row (icon, title, size, actions) and its surface. */
export const ScriptDisplay = memo(function ScriptDisplay({
  display,
  baseDir,
  stateKey,
}: {
  display: PythonDisplay;
  baseDir: string | undefined;
  stateKey?: string;
}) {
  const copy = useCopyToClipboard();
  const [asTable, setAsTable] = useRememberedState(
    stateKey && `${stateKey}:table`,
    false,
  );
  const titleId = useId();
  const kind = displayKindView(display.kind);
  const title = visibleText(displayTitle(display));
  const size = displaySize(display);
  const data = useMemo(() => chartData(display), [display]);
  const chart = data !== undefined;
  const known = display.kind === "table" || display.kind === "markdown";
  return (
    <section
      className={styles.display}
      aria-labelledby={titleId}
      data-testid="script-display"
      data-kind={display.kind}
    >
      <div className={styles.caption}>
        <Icon icon={kind.icon} size="sm" tone="muted" />
        <span id={titleId} className={styles.title} title={title}>
          {title}
        </span>
        {size && <span className={styles.size}>{size}</span>}
        <span className={styles.actions}>
          {chart && (
            <Tooltip content="Show as table">
              <IconButton
                icon={asTable ? ChartColumn : Table2}
                size="sm"
                variant="ghost"
                aria-label="Show as table"
                aria-pressed={asTable}
                onClick={() => setAsTable((value) => !value)}
              />
            </Tooltip>
          )}
          {(known || isChartKind(display.kind)) && (
            <Tooltip content="Copy">
              <IconButton
                icon={Copy}
                size="sm"
                variant="ghost"
                aria-label={`Copy ${title}`}
                onClick={() => copy(displayCopyText(display))}
              />
            </Tooltip>
          )}
          <OpenFullDataButton
            display={display}
            title={title}
            baseDir={baseDir}
          />
        </span>
      </div>
      <DisplayBody
        display={display}
        data={data}
        asTable={asTable}
        stateKey={stateKey}
      />
    </section>
  );
});

/** The displays of a run in index order: the first three, then "Show N more". */
export function ScriptDisplays({
  displays,
  total,
  baseDir,
  runKey,
}: {
  displays: readonly PythonDisplay[];
  total?: number;
  /** The run's workspace folder; "Open full data" only opens files inside it. */
  baseDir: string | undefined;
  /** Identifies the run (its tool call); keys the user's choices so they survive a remount. */
  runKey?: string;
}) {
  const stateKey = runKey ? `script-displays:${runKey}` : undefined;
  const [all, setAll] = useRememberedState(
    stateKey && `${stateKey}:all`,
    false,
  );
  const shown = all ? displays : displays.slice(0, DISPLAYS_SHOWN);
  const hidden = displays.length - shown.length;
  const missing = Math.max(0, (total ?? displays.length) - displays.length);
  if (displays.length === 0 && missing === 0) return null;
  return (
    <div className={styles.displays} data-testid="script-displays">
      {shown.map((display) => (
        <ScriptDisplay
          key={display.index}
          display={display}
          baseDir={baseDir}
          stateKey={stateKey && `${stateKey}:${display.index}`}
        />
      ))}
      {hidden > 0 && (
        <button
          type="button"
          className={styles.linkButton}
          onClick={() => setAll(true)}
        >
          Show {hidden} more {hidden === 1 ? "display" : "displays"}
        </button>
      )}
      {missing > 0 && (
        <p className={styles.note}>
          {displays.length === 0
            ? `${missing} ${missing === 1 ? "display" : "displays"} not shown`
            : `Showing ${displays.length} of ${
                displays.length + missing
              } displays`}
        </p>
      )}
    </div>
  );
}
