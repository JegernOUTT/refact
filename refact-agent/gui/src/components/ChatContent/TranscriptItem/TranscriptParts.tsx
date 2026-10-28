import React from "react";
import { ChevronDown, Copy } from "lucide-react";
import { IconButton } from "../../ui/Button";
import { useTranscriptCollapse } from "./useTranscriptCollapse";
import { useTranscriptAnchoredToggle } from "./useTranscriptAnchoredToggle";
import { formatRawValue } from "./formatRawValue";
import styles from "./TranscriptParts.module.css";

export interface TranscriptFact {
  label: React.ReactNode;
  value: React.ReactNode;
  title?: string;
}

export interface TranscriptFactsProps {
  facts: readonly TranscriptFact[];
  className?: string;
}

export function TranscriptFacts({ facts, className }: TranscriptFactsProps) {
  if (facts.length === 0) return null;
  return (
    <dl className={className ?? styles.facts} data-testid="transcript-facts">
      {facts.map((fact, index) => (
        <div className={styles.fact} key={index}>
          <dt className={styles.factLabel}>{fact.label}</dt>
          <dd className={styles.factValue} title={fact.title}>
            {fact.value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

export interface TranscriptClampProps {
  children: React.ReactNode;
  lineCount?: number;
  defaultExpanded?: boolean;
}

export function TranscriptClamp({
  children,
  lineCount,
  defaultExpanded = false,
}: TranscriptClampProps) {
  const viewportRef = React.useRef<HTMLDivElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
  const [expanded, setExpanded] = React.useState(defaultExpanded);
  const [overflowing, setOverflowing] = React.useState(false);
  const toggle = useTranscriptAnchoredToggle(viewportRef);

  React.useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const content = contentRef.current;
    if (!viewport || !content) return;
    const measure = () => {
      if (expanded) return;
      setOverflowing(content.scrollHeight > viewport.clientHeight);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    return () => observer.disconnect();
  }, [expanded]);

  const clamped = !expanded;
  const showAllLabel =
    lineCount !== undefined ? `Show all · ${lineCount} lines` : "Show all";

  return (
    <div
      className={styles.clamp}
      data-clamped={clamped}
      data-testid="transcript-clamp"
    >
      <div className={styles.clampViewport} ref={viewportRef}>
        <div ref={contentRef}>{children}</div>
        {overflowing ? <div aria-hidden className={styles.clampFade} /> : null}
      </div>
      {overflowing ? (
        <button
          className={styles.linkButton}
          type="button"
          onPointerDown={toggle.prepare}
          onClick={(event) =>
            toggle.run(event.currentTarget, () => setExpanded((v) => !v))
          }
        >
          {expanded ? "Show less" : showAllLabel}
        </button>
      ) : null}
    </div>
  );
}

export interface TranscriptRawBlock {
  label: string;
  value: unknown;
}

export interface TranscriptRawProps {
  blocks: readonly TranscriptRawBlock[];
  label?: string;
  defaultOpen?: boolean;
}

function RawBlock({ block }: { block: TranscriptRawBlock }) {
  const text = React.useMemo(() => formatRawValue(block.value), [block.value]);
  const handleCopy = React.useCallback(() => {
    void navigator.clipboard.writeText(text).catch(() => undefined);
  }, [text]);
  return (
    <div className={styles.rawBlock}>
      <div className={styles.rawHeader}>
        <span>{block.label}</span>
        <IconButton
          aria-label={`Copy ${block.label}`}
          icon={Copy}
          size="sm"
          variant="ghost"
          onClick={handleCopy}
        />
      </div>
      <pre className={styles.rawPre}>{text}</pre>
    </div>
  );
}

export function TranscriptRaw({
  blocks,
  label = "Raw",
  defaultOpen = false,
}: TranscriptRawProps) {
  const [open, setOpen] = React.useState(defaultOpen);
  const collapse = useTranscriptCollapse(open);
  const toggle = useTranscriptAnchoredToggle(collapse.panelRef);
  const bodyId = React.useId();
  if (blocks.length === 0) return null;

  return (
    <div className={styles.raw} data-open={open} data-testid="transcript-raw">
      <button
        aria-controls={bodyId}
        aria-expanded={open}
        className={styles.rawToggle}
        type="button"
        onPointerDown={toggle.prepare}
        onClick={(event) =>
          toggle.run(event.currentTarget, () => setOpen((v) => !v))
        }
      >
        {label}
        <ChevronDown aria-hidden className={styles.rawChevron} />
      </button>
      {collapse.mounted ? (
        <div
          className={styles.rawGrid}
          data-expanded={collapse.expanded}
          id={bodyId}
          ref={collapse.panelRef}
          onTransitionEnd={collapse.onTransitionEnd}
        >
          <div className={styles.rawShell}>
            <div className={styles.rawBlocks}>
              {blocks.map((block, index) => (
                <RawBlock block={block} key={`${block.label}-${index}`} />
              ))}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
