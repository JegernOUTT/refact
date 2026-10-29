import React from "react";
import classNames from "classnames";
import { ChevronDown, LoaderCircle } from "lucide-react";
import { Tooltip } from "../../ui/Tooltip";
import { useStoredOpen } from "../useStoredOpen";
import {
  type TranscriptStatus,
  isQuietStatus,
  transcriptStatusLabel,
} from "./statusVocabulary";
import { useTranscriptCollapse } from "./useTranscriptCollapse";
import { useCollapsedFocusGuard } from "./useCollapsedFocusGuard";
import { useTranscriptAnchoredToggle } from "./useTranscriptAnchoredToggle";
import {
  TranscriptFacts,
  TranscriptRaw,
  type TranscriptFact,
  type TranscriptRawBlock,
} from "./TranscriptParts";
import {
  DEFAULT_SECTION_LABELS,
  TRANSCRIPT_SECTION_ORDER,
  type TranscriptSectionId,
} from "./transcriptSections";
import styles from "./TranscriptItem.module.css";

export type TranscriptSeparator = "dot" | "arrow";

export interface TranscriptItemProps
  extends Omit<React.HTMLAttributes<HTMLElement>, "title" | "children"> {
  icon?: React.ReactNode;
  verb?: React.ReactNode;
  target?: React.ReactNode;
  separator?: TranscriptSeparator;
  meta?: React.ReactNode;
  status?: TranscriptStatus;
  statusLabel?: React.ReactNode;
  statusDetail?: React.ReactNode;
  quickAction?: React.ReactNode;
  summary?: readonly TranscriptFact[] | React.ReactNode;
  input?: React.ReactNode;
  output?: React.ReactNode;
  artifacts?: React.ReactNode;
  error?: React.ReactNode;
  actions?: React.ReactNode;
  raw?: readonly TranscriptRawBlock[];
  sectionLabels?: Partial<Record<TranscriptSectionId, string>>;
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  openKey?: string;
  expandable?: boolean;
  animate?: boolean;
  rowTooltip?: React.ReactNode;
  rowClassName?: string;
  panelClassName?: string;
  keepMounted?: boolean;
  panelRef?: React.Ref<HTMLDivElement>;
}

const SEPARATORS: Record<TranscriptSeparator, string> = {
  dot: "·",
  arrow: "→",
};

function hasContent(node: unknown): boolean {
  if (node === null || node === undefined || node === false) return false;
  if (node === "") return false;
  if (Array.isArray(node)) return node.length > 0;
  return true;
}

function isFactList(
  value: TranscriptItemProps["summary"],
): value is readonly TranscriptFact[] {
  return Array.isArray(value);
}

function assignRef<T>(ref: React.Ref<T> | undefined, value: T | null) {
  if (!ref) return;
  if (typeof ref === "function") ref(value);
  else (ref as React.MutableRefObject<T | null>).current = value;
}

function stopRowEvent(event: React.SyntheticEvent) {
  event.stopPropagation();
}

function useTranscriptOpen(
  openKey: string | undefined,
  defaultOpen: boolean,
  open: boolean | undefined,
  onOpenChange: ((open: boolean) => void) | undefined,
): [boolean, () => void] {
  const [storedOpen, , setStoredOpen] = useStoredOpen(openKey, defaultOpen);
  const isControlled = open !== undefined;
  const isOpen = isControlled ? open : storedOpen;
  const toggle = React.useCallback(() => {
    const next = !isOpen;
    if (!isControlled) setStoredOpen(next);
    onOpenChange?.(next);
  }, [isControlled, isOpen, onOpenChange, setStoredOpen]);
  return [isOpen, toggle];
}

export function TranscriptItem({
  icon,
  verb,
  target,
  separator = "dot",
  meta,
  status = "done",
  statusLabel,
  statusDetail,
  quickAction,
  summary,
  input,
  output,
  artifacts,
  error,
  actions,
  raw,
  sectionLabels,
  defaultOpen = false,
  open,
  onOpenChange,
  openKey,
  expandable,
  animate = true,
  rowTooltip,
  rowClassName,
  panelClassName,
  keepMounted = false,
  panelRef,
  className,
  ...rootProps
}: TranscriptItemProps) {
  const labels = { ...DEFAULT_SECTION_LABELS, ...sectionLabels };
  const sections: Partial<Record<TranscriptSectionId, React.ReactNode>> = {
    summary: isFactList(summary) ? (
      summary.length > 0 ? (
        <TranscriptFacts facts={summary} />
      ) : null
    ) : (
      summary
    ),
    input,
    output,
    artifacts,
    error: hasContent(error) ? (
      <div className={styles.error} role="alert">
        {error}
      </div>
    ) : null,
    actions: hasContent(actions) ? (
      <div className={styles.actions}>{actions}</div>
    ) : null,
    raw:
      raw && raw.length > 0 ? (
        <TranscriptRaw blocks={raw} label={labels.raw} />
      ) : null,
  };
  const visibleSections = TRANSCRIPT_SECTION_ORDER.filter((id) =>
    hasContent(sections[id]),
  );
  const canExpand = expandable ?? visibleSections.length > 0;

  const [isOpen, toggleOpen] = useTranscriptOpen(
    openKey,
    defaultOpen,
    open,
    onOpenChange,
  );
  const hasSections = visibleSections.length > 0;
  const effectiveOpen = canExpand ? isOpen : hasSections && open === true;
  const collapse = useTranscriptCollapse(effectiveOpen, animate);
  const internalPanelRef = collapse.panelRef;
  const [panelNode, setPanelNode] = React.useState<HTMLDivElement | null>(null);
  const setPanelRef = React.useCallback(
    (node: HTMLDivElement | null) => {
      internalPanelRef.current = node;
      setPanelNode(node);
      assignRef(panelRef, node);
    },
    [internalPanelRef, panelRef],
  );
  const guardRef = React.useMemo(() => ({ current: panelNode }), [panelNode]);
  useCollapsedFocusGuard(guardRef, !effectiveOpen);
  const renderPanel = hasSections && (keepMounted || collapse.mounted);
  const anchoredToggle = useTranscriptAnchoredToggle(collapse.panelRef);
  const panelId = React.useId();
  const pillId = React.useId();

  const handleToggle = React.useCallback(
    (trigger: HTMLElement) => {
      anchoredToggle.run(trigger, toggleOpen);
    },
    [anchoredToggle, toggleOpen],
  );

  const toggleRef = React.useRef<HTMLButtonElement>(null);
  const handleRowClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (!canExpand || !toggleRef.current) return;
    if (
      event.target instanceof Node &&
      toggleRef.current.contains(event.target)
    )
      return;
    handleToggle(toggleRef.current);
  };

  const running = status === "running";
  const pillVisible = !isQuietStatus(status) || hasContent(statusLabel);
  const pillText = statusLabel ?? transcriptStatusLabel(status);
  const pill = (
    <span
      className={styles.pill}
      data-tone={status}
      data-testid="transcript-status-pill"
      id={pillId}
      data-visible={pillVisible}
    >
      {pillText}
    </span>
  );

  const headContent = (
    <>
      <span className={styles.icon} data-testid="transcript-icon">
        <span className={styles.iconLayer} data-active={!running}>
          {icon}
        </span>
        <span className={styles.iconLayer} data-active={running}>
          {running ? (
            <LoaderCircle aria-hidden className="rf-spin" focusable="false" />
          ) : null}
        </span>
      </span>
      <span className={styles.title} data-testid="transcript-title">
        {hasContent(verb) ? (
          <span className={styles.verb} data-testid="transcript-verb">
            {verb}
          </span>
        ) : null}
        {hasContent(verb) && hasContent(target) ? (
          <span aria-hidden className={styles.separator}>
            {SEPARATORS[separator]}
          </span>
        ) : null}
        {hasContent(target) ? (
          <span className={styles.target} data-testid="transcript-target">
            {target}
          </span>
        ) : null}
      </span>
      {hasContent(meta) ? (
        <span className={styles.meta} data-testid="transcript-meta">
          {meta}
        </span>
      ) : null}
      <span className={styles.spacer} />
    </>
  );

  const head = canExpand ? (
    <button
      aria-controls={panelId}
      aria-describedby={pillVisible ? pillId : undefined}
      aria-expanded={effectiveOpen}
      className={styles.toggle}
      data-testid="transcript-toggle"
      ref={toggleRef}
      type="button"
      onClick={(event) => handleToggle(event.currentTarget)}
      onKeyDown={anchoredToggle.prepare}
      onPointerDown={anchoredToggle.prepare}
    >
      {headContent}
    </button>
  ) : (
    <div className={styles.toggle}>{headContent}</div>
  );

  const row = (
    <div
      className={classNames(styles.row, rowClassName)}
      data-expandable={canExpand}
      data-testid="transcript-row"
      data-transcript-row=""
      onClick={canExpand ? handleRowClick : undefined}
    >
      {rowTooltip ? <Tooltip content={rowTooltip}>{head}</Tooltip> : head}
      {pillVisible && hasContent(statusDetail) ? (
        <Tooltip content={statusDetail}>{pill}</Tooltip>
      ) : pillVisible ? (
        pill
      ) : null}
      {hasContent(quickAction) ? (
        <span
          className={styles.quickAction}
          onClick={stopRowEvent}
          onKeyDown={stopRowEvent}
          onPointerDown={stopRowEvent}
        >
          {quickAction}
        </span>
      ) : null}
      {canExpand ? (
        <ChevronDown
          aria-hidden
          className={styles.chevron}
          data-testid="transcript-chevron"
        />
      ) : null}
    </div>
  );

  return (
    <section
      className={classNames(styles.root, className)}
      data-open={effectiveOpen}
      data-status={status}
      data-transcript-item="true"
      data-transcript-status={status}
      {...rootProps}
    >
      {row}
      {renderPanel ? (
        <div
          className={classNames("rf-expand-grid", styles.panelGrid)}
          data-expanded={collapse.expanded}
          data-open={effectiveOpen}
          data-testid="transcript-panel"
          id={panelId}
          ref={setPanelRef}
          onTransitionEnd={collapse.onTransitionEnd}
        >
          <div className={styles.panelShell}>
            <div className={classNames(styles.panel, panelClassName)}>
              {visibleSections.map((id) => (
                <div
                  aria-label={labels[id] || undefined}
                  role="group"
                  className={styles.section}
                  data-section={id}
                  key={id}
                >
                  {id !== "raw" && labels[id] ? (
                    <h4 className={styles.sectionLabel}>{labels[id]}</h4>
                  ) : null}
                  <div className={styles.sectionBody}>{sections[id]}</div>
                </div>
              ))}
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}

export default TranscriptItem;
