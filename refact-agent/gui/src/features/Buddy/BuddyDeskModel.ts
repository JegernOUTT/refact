import type {
  DeskActionBatchEntry,
  OperatorRecord,
} from "../../services/refact/buddyOperator";
import type { WorkItem, WorkItemState } from "./operatorTypes";
import type { BuddyReportsView } from "./sections/buddyReportsLink";

export const DESK_VIEWS: readonly BuddyReportsView[] = [
  "needs_you",
  "working_on",
  "delivered",
  "watching",
];

export const DESK_VIEW_LABELS: Record<BuddyReportsView, string> = {
  needs_you: "Needs you",
  working_on: "Working on",
  delivered: "Delivered",
  watching: "Watching",
};

export const DEFAULT_DESK_VIEW: BuddyReportsView = "needs_you";

export function isDeskView(value: string): value is BuddyReportsView {
  return (DESK_VIEWS as readonly string[]).includes(value);
}

export const VIEW_FOR_STATE: Record<WorkItemState, BuddyReportsView> = {
  needs_attention: "needs_you",
  blocked: "needs_you",
  working: "working_on",
  delivered: "delivered",
  cancelled: "delivered",
  watching: "watching",
};

export const VIEW_DEFAULT_STATE: Record<BuddyReportsView, WorkItemState> = {
  needs_you: "needs_attention",
  working_on: "working",
  delivered: "delivered",
  watching: "watching",
};

export const EMPTY_VIEW_COPY: Record<BuddyReportsView, string> = {
  needs_you: "Nothing needs you right now.",
  working_on: "Nothing is in progress right now.",
  delivered: "No delivered reports in this project yet.",
  watching: "Nothing is being watched right now.",
};

export type DeskAttention = "all" | "unread" | "snoozed" | "unsnoozed";

export const DESK_ATTENTION_OPTIONS: { value: DeskAttention; label: string }[] =
  [
    { value: "unsnoozed", label: "Active reports" },
    { value: "unread", label: "Unread reports" },
    { value: "snoozed", label: "Snoozed reports" },
    { value: "all", label: "All reports, including snoozed" },
  ];

export function isDeskAttention(value: string): value is DeskAttention {
  return DESK_ATTENTION_OPTIONS.some((option) => option.value === value);
}

export type DeskListMode = "latest" | "history";

export const DESK_LIST_MODES: { value: DeskListMode; label: string }[] = [
  { value: "latest", label: "Latest by subject" },
  { value: "history", label: "Run history" },
];

export const NO_RESULT_YET = "No result yet";
export const NO_ACTION = "No action to take";
export const ACTION_STATUS_UNAVAILABLE = "Action status unavailable";
const ROW_GROUP_SIZE = 50;
export const ROW_SELECTOR = "button[data-desk-row]";
export const CANCELLED_GROUP = "cancelled";

export type RowActions =
  | { mode: "per_row" }
  | {
      mode: "batch";
      entry: DeskActionBatchEntry | undefined;
      loaded: boolean;
      pending: boolean;
      failed: boolean;
    };

export type ChunkActions = Record<string, RowActions>;

export const PENDING_ROW_ACTIONS: RowActions = {
  mode: "batch",
  entry: undefined,
  loaded: false,
  pending: true,
  failed: false,
};

export type DeskRecordGroup = {
  key: string;
  head: OperatorRecord<WorkItem>;
  others: OperatorRecord<WorkItem>[];
};

function deskSubjectKey(item: WorkItem): string | null {
  const kind = item.kind.trim();
  const resourceKind = item.subject.resource_kind.trim();
  const resourceId = item.subject.resource_id.trim();
  if (kind === "" || resourceKind === "" || resourceId === "") return null;
  return JSON.stringify([
    item.project.project_id,
    item.project.canonical_root,
    kind,
    resourceKind,
    resourceId,
  ]);
}

export function groupDeskRecords(
  records: OperatorRecord<WorkItem>[],
): DeskRecordGroup[] {
  const groups = new Map<string, DeskRecordGroup>();
  for (const record of records) {
    const key = deskSubjectKey(record.value) ?? `work:${record.value.id}`;
    const group = groups.get(key);
    if (group) group.others.push(record);
    else groups.set(key, { key, head: record, others: [] });
  }
  return [...groups.values()];
}

export function chunkWork(
  records: OperatorRecord<WorkItem>[],
  endpointIdentity: string,
): { key: string; items: WorkItem[] }[] {
  return Array.from(
    { length: Math.ceil(records.length / ROW_GROUP_SIZE) },
    (_, index) => {
      const items = records
        .slice(index * ROW_GROUP_SIZE, (index + 1) * ROW_GROUP_SIZE)
        .map(({ value }) => value);
      return {
        key: JSON.stringify([
          endpointIdentity,
          ...items.map((value) => [
            value.id,
            value.project.project_id,
            value.current_artifact_id,
            value.current_artifact_revision,
          ]),
        ]),
        items,
      };
    },
  );
}

export function sameChunkActions(
  left: ChunkActions | undefined,
  right: ChunkActions,
): boolean {
  if (!left) return false;
  const keys = Object.keys(right);
  if (Object.keys(left).length !== keys.length) return false;
  return keys.every((id) => {
    const a = left[id];
    const b = right[id];
    if (!(id in left) || a.mode !== b.mode) return false;
    if (a.mode === "per_row" || b.mode === "per_row") return true;
    return (
      a.entry === b.entry &&
      a.loaded === b.loaded &&
      a.pending === b.pending &&
      a.failed === b.failed
    );
  });
}

export function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}
