import React from "react";
import { Bug, CircleCheck, ShieldAlert, TriangleAlert } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Button } from "../../components/ui";
import type { BuddyRuntimeEvent } from "./types";
import {
  BUDDY_FAILURE_LABELS,
  BUDDY_TERMS,
  BUDDY_VERBS,
  BuddyDisclosure,
  BuddyEmptyState,
  BuddyRow,
  BuddySection,
  BuddyTimestamp,
  buddyFailureLabel,
  buddyWorkflowLabel,
  parseBuddyTime,
  useBuddyPersistentState,
  type BuddyStatusTone,
} from "./ui";
import { HISTORY_PAGE_SIZE } from "./buddyHistory";
import styles from "./BuddyRecentErrorsPanel.module.css";

export type RecentBuddyError = BuddyRuntimeEvent & {
  occurrences?: number;
  dismissedAny?: boolean;
  dismissedAll?: boolean;
  relatedIds?: string[];
};

export interface BuddyWarningRow {
  event: RecentBuddyError;
  detail: string | null;
  count: number;
}

const CODE_PREFIX = /^([a-z][a-z0-9]*(?:_[a-z0-9]+)+):\s*(.+)$/s;
const WORKFLOW_PREFIX = /^([a-z][a-z0-9]*(?:_[a-z0-9]+)+)(\s.+)$/s;

const BRACKET_CODE = /\[[a-z0-9_]+(?::[a-z0-9_]+)*\]\s*/gi;

function clean(value: string | null | undefined): string {
  return (value ?? "").trim().replace(/\s+/g, " ");
}

function firstSentence(value: string): string {
  const match = /^(.+?[.!?])(\s|$)/.exec(value);
  return match ? match[1] : value;
}

function knownFailureKey(value: string): string | null {
  const key = value.toLowerCase().replace(/[\s-]+/g, "_");
  return BUDDY_FAILURE_LABELS[key] ? key : null;
}

function untruncated(event: RecentBuddyError, raw: string): string {
  const fuller = [event.failure_summary, event.description]
    .map(clean)
    .find((text) => text.length > raw.length && text.startsWith(raw));
  return fuller ?? raw;
}

function warningTitle(raw: string, detail: string | null): string {
  const prefixed = CODE_PREFIX.exec(raw);
  if (prefixed) return `${buddyFailureLabel(prefixed[1])}: ${prefixed[2]}`;
  const workflow = WORKFLOW_PREFIX.exec(raw);
  if (workflow) return `${buddyWorkflowLabel(workflow[1])}${workflow[2]}`;
  const key = knownFailureKey(raw);
  if (!key) return raw || buddyFailureLabel(null);
  const label = buddyFailureLabel(key);
  const extra = clean(detail);
  if (!extra || label.toLowerCase() === raw.toLowerCase()) return label;
  return `${label}: ${firstSentence(extra)}`;
}

function warningLeading(
  event: RecentBuddyError,
  acknowledged: boolean,
): { icon: LucideIcon; tone: BuddyStatusTone } {
  if (acknowledged) return { icon: CircleCheck, tone: "success" };
  if (event.priority === "critical")
    return { icon: ShieldAlert, tone: "danger" };
  if (event.priority === "high")
    return { icon: TriangleAlert, tone: "warning" };
  return { icon: Bug, tone: "default" };
}

function technicalText(
  event: RecentBuddyError,
  raw: string,
  title: string,
  detail: string | null,
): string {
  const parts = [raw !== title ? raw : null, clean(detail) || null];
  if (event.failure_category) parts.push(`Category: ${event.failure_category}`);
  if (event.chat_id) parts.push(`Chat: ${event.chat_id}`);
  return parts.filter(Boolean).join("\n");
}

const WarningItem: React.FC<{
  row: BuddyWarningRow;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  actions?: React.ReactNode;
}> = ({ row: { event, detail, count }, open, onOpenChange, actions }) => {
  const acknowledged = Boolean(event.dismissedAll ?? event.dismissed);
  const raw = clean(event.title);
  const title = warningTitle(
    untruncated(event, raw).replace(BRACKET_CODE, "").trim(),
    detail?.replace(BRACKET_CODE, "") ?? null,
  );
  const technical = technicalText(event, raw, title, detail);
  const hasTime = parseBuddyTime(event.created_at) !== null;
  return (
    <li
      className={styles.item}
      data-priority={event.priority}
      data-acknowledged={acknowledged ? "true" : undefined}
    >
      <BuddyRow
        title={title}
        leading={warningLeading(event, acknowledged)}
        meta={[
          hasTime ? <BuddyTimestamp value={event.created_at} /> : null,
          count > 1 ? `×${count}` : null,
          acknowledged ? "Dismissed" : null,
        ]}
        actions={actions}
      />
      {technical ? (
        <BuddyDisclosure
          label={BUDDY_TERMS.technicalDetails}
          open={open}
          onOpenChange={onOpenChange}
          lazy
          className={styles.details}
        >
          <pre className={styles.raw}>{technical}</pre>
        </BuddyDisclosure>
      ) : null}
    </li>
  );
};

export const BuddyRecentWarnings: React.FC<{
  rows: BuddyWarningRow[];
  persistKey: string;
  testId?: string;
  renderActions?: (event: RecentBuddyError) => React.ReactNode;
}> = ({ rows, persistKey, testId, renderActions }) => {
  const [visibleCount, setVisibleCount] = useBuddyPersistentState(
    `${persistKey}.visible`,
    HISTORY_PAGE_SIZE,
  );
  const [openIds, setOpenIds] = useBuddyPersistentState<string[]>(
    `${persistKey}.openIds`,
    [],
  );
  React.useEffect(() => {
    const ids = new Set(rows.map((row) => row.event.id));
    const pruned = openIds.filter((id) => ids.has(id));
    if (pruned.length !== openIds.length) setOpenIds(pruned);
  }, [rows, openIds, setOpenIds]);
  const visible = rows.slice(0, visibleCount);
  return (
    <BuddySection
      title="Recent warnings"
      icon={TriangleAlert}
      count={rows.length}
      testId={testId}
      footer={
        visible.length < rows.length ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() =>
              setVisibleCount((value) => value + HISTORY_PAGE_SIZE)
            }
          >
            {BUDDY_VERBS.showMore}
          </Button>
        ) : null
      }
    >
      {rows.length === 0 ? (
        <BuddyEmptyState
          compact
          icon={CircleCheck}
          title="No warnings in the last six hours"
        />
      ) : (
        <ul aria-label="Recent warnings" className={styles.list}>
          {visible.map((row) => (
            <WarningItem
              key={row.event.id}
              row={row}
              open={openIds.includes(row.event.id)}
              onOpenChange={(open) =>
                setOpenIds((ids) =>
                  open
                    ? [...ids, row.event.id]
                    : ids.filter((id) => id !== row.event.id),
                )
              }
              actions={renderActions?.(row.event)}
            />
          ))}
        </ul>
      )}
    </BuddySection>
  );
};

interface BuddyRecentErrorsPanelProps {
  recentErrors: RecentBuddyError[];
  onInvestigate: (event: RecentBuddyError) => void | Promise<void>;
  onDismiss: (event: RecentBuddyError) => void | Promise<void>;
}

export const BuddyRecentErrorsPanel: React.FC<BuddyRecentErrorsPanelProps> = ({
  recentErrors,
  onInvestigate,
  onDismiss,
}) => (
  <BuddyRecentWarnings
    testId="buddy-recent-errors-panel"
    persistKey="buddy.activity.legacyWarnings"
    rows={recentErrors.map((event) => ({
      event,
      detail: event.failure_summary ?? event.description ?? null,
      count: event.occurrences ?? 1,
    }))}
    renderActions={(event) => (
      <>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={() => void onInvestigate(event)}
        >
          Investigate
        </Button>
        {!(event.dismissedAll ?? event.dismissed) && (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => void onDismiss(event)}
          >
            {BUDDY_VERBS.dismiss}
          </Button>
        )}
      </>
    )}
  />
);
