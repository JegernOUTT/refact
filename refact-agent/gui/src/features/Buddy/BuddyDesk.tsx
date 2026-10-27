import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  ChevronRight,
  ClipboardList,
  FileSearch,
  Inbox,
  RefreshCw,
} from "lucide-react";
import { Button, Icon, Surface, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  useGetBuddyDeskActionsQuery,
  useGetBuddyDeskActionsBatchQuery,
  useGetBuddyOperatorWorkQuery,
  useGetBuddyOperatorWorkItemQuery,
  buddyOperatorIdentity,
  type DeskActionBatchEntry,
  type OperatorRecord,
} from "../../services/refact/buddyOperator";
import type { WorkItem, WorkItemState } from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { useConfig } from "../../hooks/useConfig";
import { BuddyDeskDetail } from "./BuddyDeskDetail";
import { headline } from "./buddyUtils";
import styles from "./BuddyDesk.module.css";

const views = ["Needs you", "Working on", "Delivered", "Watching"] as const;
type WorkView = (typeof views)[number];
const viewQuery = {
  "Needs you": "needs_you",
  "Working on": "working_on",
  Delivered: "delivered",
  Watching: "watching",
} as const;

const DEFAULT_VIEW: WorkView = "Needs you";

const viewForState: Record<WorkItemState, WorkView> = {
  needs_attention: "Needs you",
  blocked: "Needs you",
  working: "Working on",
  delivered: "Delivered",
  cancelled: "Delivered",
  watching: "Watching",
};

const emptyView: Record<WorkView, string> = {
  "Needs you": "Nothing needs you right now.",
  "Working on": "Nothing is in progress right now.",
  Delivered: "No delivered work is saved in this project.",
  Watching: "Nothing is being watched right now.",
};

const NO_RESULT_YET = "No result yet";
const NO_ACTION = "No action to take";
const ACTION_STATUS_UNAVAILABLE = "Action status unavailable";
const ROW_GROUP_SIZE = 50;

const opaqueIdentifier = /[0-9a-f]{24,}/i;
const ROW_SELECTOR = "button[data-desk-row]";
const CANCELLED_GROUP = "cancelled";

const listModes = ["Latest by subject", "Run history"] as const;
type ListMode = (typeof listModes)[number];

type DeskRecordGroup = {
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

function groupDeskRecords(
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

function chunkWork(
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

function sameChunkActions(
  left: ChunkActions | undefined,
  right: Record<string, RowActions>,
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

function visibleRows(list: HTMLElement | null): HTMLButtonElement[] {
  return [
    ...(list?.querySelectorAll<HTMLButtonElement>(ROW_SELECTOR) ?? []),
  ].filter((row) => !row.closest("details:not([open])"));
}

function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}

function meaningfulText(value: string | null | undefined): string | null {
  const text = value?.trim() ?? "";
  if (text === "" || opaqueIdentifier.test(text)) return null;
  return text;
}

function comparableText(value: string): string {
  return value
    .toLocaleLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .replace(/\b(?:a|an|the|issue|problem|finding|report|review)\b/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

function problemSummary(item: WorkItem): string | null {
  const summary = meaningfulText(item.problem_summary);
  if (!summary) return null;
  const title = comparableText(item.title);
  const comparableSummary = comparableText(summary);
  if (
    comparableSummary === "" ||
    comparableSummary === title ||
    comparableSummary.includes(title) ||
    title.includes(comparableSummary)
  )
    return null;
  return summary;
}

function severityLabel(item: WorkItem): string | null {
  if (!item.severity || item.severity === "unknown") return null;
  return `${humanizeIdentifier(item.severity)} severity`;
}

function changedAt(value: string): string | null {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  const minutes = Math.round((Date.now() - parsed) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

function changedAtAbsolute(value: string): string | null {
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return null;
  return new Date(parsed).toLocaleString();
}

function WorkRow({
  item,
  endpointIdentity,
  showState,
  showActionVerb,
  rowActions,
  selected,
  onOpen,
  onActionVerb,
}: {
  item: WorkItem;
  endpointIdentity: string;
  showState: boolean;
  showActionVerb: boolean;
  rowActions: RowActions;
  selected: boolean;
  onOpen: (item: WorkItem, trigger: HTMLButtonElement) => void;
  onActionVerb: (id: string, verb: string) => void;
}) {
  const artifactId = item.current_artifact_id;
  const { currentData, isFetching, error } = useGetBuddyDeskActionsQuery(
    {
      id: item.id,
      project_id: item.project.project_id,
      endpoint_identity: endpointIdentity,
    },
    { skip: !artifactId || rowActions.mode !== "per_row" },
  );
  const actions =
    rowActions.mode === "per_row"
      ? currentData?.data
      : rowActions.entry?.actions;
  const action = actions?.find(
    (candidate) =>
      candidate.artifact_id === artifactId &&
      candidate.artifact_revision === item.current_artifact_revision &&
      candidate.spec.project.canonical_root === item.project.canonical_root,
  );
  const failed =
    rowActions.mode === "per_row"
      ? Boolean(error)
      : rowActions.failed ||
        (rowActions.loaded &&
          (rowActions.entry === undefined || rowActions.entry.error !== null));
  const pending =
    rowActions.mode === "per_row"
      ? isFetching && !currentData
      : rowActions.pending;
  let verb: string | null = NO_ACTION;
  if (!artifactId) verb = NO_RESULT_YET;
  else if (failed) verb = ACTION_STATUS_UNAVAILABLE;
  else if (pending) verb = null;
  else if (action) verb = action.label;
  const changed = changedAt(item.last_material_change_at);
  const absolute = changedAtAbsolute(item.last_material_change_at);
  const absoluteTimeId = useId();
  const severity = severityLabel(item);
  const scope = meaningfulText(item.scope);
  const summary = problemSummary(item);
  useEffect(() => {
    if (verb !== null) onActionVerb(item.id, verb);
  }, [item.id, onActionVerb, verb]);
  return (
    <li>
      <button
        data-desk-row={item.id}
        className={styles.item}
        aria-current={selected ? "true" : undefined}
        aria-describedby={absolute ? absoluteTimeId : undefined}
        onClick={(event) => onOpen(item, event.currentTarget)}
      >
        <span className={styles.itemContent}>
          <span className={styles.titleLine}>
            {severity && (
              <span
                aria-label={severity}
                className={styles.severity}
                data-severity={item.severity}
                role="img"
              />
            )}
            <Text
              size="2"
              weight="medium"
              className={styles.name}
              data-testid="buddy-desk-row-title"
            >
              {headline(item.title)}
            </Text>
          </span>
          {summary && (
            <Text size="1" className={styles.problemSummary}>
              {summary}
            </Text>
          )}
          <span className={styles.itemMeta}>
            {scope && (
              <Text size="1" className={styles.scope}>
                {scope}
              </Text>
            )}
            {scope && showActionVerb && verb !== null && (
              <span aria-hidden className={styles.metaSeparator}>
                ·
              </span>
            )}
            {showActionVerb && verb !== null && (
              <Text
                size="1"
                weight={action ? "medium" : "regular"}
                className={styles.actionVerb}
              >
                {verb}
              </Text>
            )}
            <Text
              size="1"
              color="gray"
              className={showState ? undefined : styles.srOnly}
            >
              {humanizeIdentifier(item.state)}
            </Text>
            {changed && (
              <Text size="1" color="gray" title={absolute ?? undefined}>
                {changed}
              </Text>
            )}
            {absolute && (
              <span
                id={absoluteTimeId}
                className={styles.srOnly}
                aria-hidden="true"
              >
                {absolute}
              </span>
            )}
          </span>
        </span>
        <Icon
          icon={ChevronRight}
          size="sm"
          tone="muted"
          className={styles.chevron}
        />
      </button>
    </li>
  );
}

type RowActions =
  | { mode: "per_row" }
  | {
      mode: "batch";
      entry: DeskActionBatchEntry | undefined;
      loaded: boolean;
      pending: boolean;
      failed: boolean;
    };

type ChunkActions = Record<string, RowActions>;

function ActionBatchSubscriber({
  chunkKey,
  chunk,
  endpointIdentity,
  onResult,
}: {
  chunkKey: string;
  chunk: WorkItem[];
  endpointIdentity: string;
  onResult: (key: string, result: ChunkActions | null) => void;
}) {
  const projectId = chunk[0]?.project.project_id ?? "";
  const ids = chunk
    .filter(
      (value) =>
        Boolean(value.current_artifact_id) &&
        value.project.project_id === projectId,
    )
    .map((value) => value.id);
  const batch = useGetBuddyDeskActionsBatchQuery(
    { ids, project_id: projectId, endpoint_identity: endpointIdentity },
    { skip: ids.length === 0 },
  );
  const { currentData, isFetching, error } = batch;
  useEffect(() => {
    const status =
      error && typeof error === "object" && "status" in error
        ? error.status
        : undefined;
    const perRow = status === 404 || status === 405;
    const result: ChunkActions = {};
    for (const item of chunk) {
      result[item.id] = perRow
        ? { mode: "per_row" }
        : {
            mode: "batch",
            entry: currentData?.data.find((entry) => entry.work_id === item.id),
            loaded: currentData !== undefined,
            pending: isFetching && !currentData,
            failed:
              error !== undefined || item.project.project_id !== projectId,
          };
    }
    onResult(chunkKey, result);
  }, [chunk, chunkKey, currentData, error, isFetching, onResult, projectId]);
  useEffect(() => () => onResult(chunkKey, null), [chunkKey, onResult]);
  return null;
}

const PENDING_ROW_ACTIONS: RowActions = {
  mode: "batch",
  entry: undefined,
  loaded: false,
  pending: true,
  failed: false,
};

function DeskContent({
  endpointIdentity,
  workId,
  projectId,
}: {
  endpointIdentity: string;
  workId?: string;
  projectId?: string;
}) {
  const config = useConfig();
  const [cursor, setCursor] = useState<number>();
  const [previous, setPrevious] = useState<OperatorRecord<WorkItem>[]>([]);
  const [actionVerbs, setActionVerbs] = useState<
    Partial<Record<string, string>>
  >({});
  const [view, setView] = useState<WorkView>(DEFAULT_VIEW);
  const [listMode, setListMode] = useState<ListMode>("Latest by subject");
  const [chunkActions, setChunkActions] = useState<
    Partial<Record<string, ChunkActions>>
  >({});
  const recordChunkActions = useCallback(
    (key: string, result: ChunkActions | null) => {
      setChunkActions((current) => {
        if (result === null) {
          if (!(key in current)) return current;
          return Object.fromEntries(
            Object.entries(current).filter(([entry]) => entry !== key),
          );
        }
        if (sameChunkActions(current[key], result)) return current;
        return { ...current, [key]: result };
      });
    },
    [],
  );
  const [openGroups, setOpenGroups] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const toggleGroup = useCallback((key: string, isOpen: boolean) => {
    setOpenGroups((current) => {
      if (current.has(key) === isOpen) return current;
      const next = new Set(current);
      if (isOpen) next.add(key);
      else next.delete(key);
      return next;
    });
  }, []);
  const [selectedId, setSelectedId] = useState<string | null>(workId ?? null);
  const [selectedOverride, setSelectedOverride] = useState<WorkItem | null>(
    null,
  );
  const dismissSnapshotRef = useRef<WorkItem | null>(null);
  const [expanded, setExpanded] = useState(false);
  const linkedWork = useGetBuddyOperatorWorkItemQuery(
    {
      id: workId ?? "",
      project_id: projectId,
      endpoint_identity: endpointIdentity,
    },
    { skip: !workId || !projectId, refetchOnMountOrArgChange: true },
  );
  const [search, setSearch] = useState("");
  const [attention, setAttention] = useState<
    "all" | "unread" | "snoozed" | "unsnoozed"
  >("unsnoozed");
  const listRef = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement | null>(null);
  const viewTrigger = useRef<HTMLButtonElement | null>(null);
  const restoreViewFocus = useRef(false);
  const expandButton = useRef<HTMLButtonElement>(null);
  const refreshButton = useRef<HTMLButtonElement>(null);
  const refreshStartedAt = useRef<number | null>(null);
  const [refreshPending, setRefreshPending] = useState(false);
  const [refreshNotice, setRefreshNotice] = useState(false);
  const [dismissalNotice, setDismissalNotice] = useState<string | null>(null);
  const refreshFocusPending = useRef(false);
  const snoozeFocusPending = useRef(false);
  const dismissalPendingId = useRef<string | null>(null);
  const dismissalFocusPending = useRef<{ id: string; index: number } | null>(
    null,
  );
  const dismissedSelectionId = useRef<string | null>(null);
  const snoozedSelectionId = useRef<string | null>(null);
  const {
    currentData,
    isFetching,
    isError,
    error,
    startedTimeStamp,
    fulfilledTimeStamp,
    refetch,
  } = useGetBuddyOperatorWorkQuery(
    {
      cursor,
      limit: 50,
      endpoint_identity: endpointIdentity,
      view: viewQuery[view],
      search: search.trim(),
      attention,
    },
    { refetchOnMountOrArgChange: true },
  );
  const records = useMemo(() => {
    const compatiblePrevious = currentData
      ? previous.filter(
          (record) =>
            record.value.project.project_id ===
              currentData.project.project_id &&
            record.value.project.canonical_root ===
              currentData.project.canonical_root,
        )
      : previous;
    return [
      ...new Map(
        [...compatiblePrevious, ...(currentData?.data ?? [])].map((record) => [
          record.value.id,
          record,
        ]),
      ).values(),
    ];
  }, [currentData, previous]);
  const linkedPage = linkedWork.currentData;
  const linkedValue =
    linkedPage &&
    workId !== undefined &&
    projectId !== undefined &&
    linkedPage.data.value.id === workId &&
    linkedPage.project.project_id === projectId &&
    linkedPage.data.value.project.project_id === projectId &&
    linkedPage.data.value.project.canonical_root ===
      linkedPage.project.canonical_root
      ? linkedPage.data.value
      : undefined;
  const linkedFailure =
    workId !== undefined &&
    (!projectId ||
      Boolean(linkedWork.error) ||
      (linkedPage !== undefined && linkedValue === undefined));
  const selected =
    selectedId === workId
      ? linkedValue
      : records.find((record) => record.value.id === selectedId)?.value ??
        (selectedOverride?.id === selectedId ? selectedOverride : undefined) ??
        (dismissSnapshotRef.current?.id === selectedId
          ? dismissSnapshotRef.current
          : undefined);
  const query = search.trim().toLocaleLowerCase();
  const filtered = records.filter(
    ({ value }) => viewForState[value.state] === view,
  );
  const distinctStates = new Set(filtered.map(({ value }) => value.state));
  const primary = filtered.filter(({ value }) => value.state !== "cancelled");
  const cancelled = filtered.filter(({ value }) => value.state === "cancelled");
  const groups = groupDeskRecords(primary);
  const repeatedSubjects = groups.filter((group) => group.others.length > 0);
  const actionChunks = chunkWork(filtered, endpointIdentity);
  const chunkByWork = new Map(
    actionChunks.flatMap((chunk) =>
      chunk.items.map((value) => [value.id, chunk.key] as const),
    ),
  );
  const rowActionsFor = (id: string): RowActions => {
    const chunkKey = chunkByWork.get(id);
    const chunk = chunkKey === undefined ? undefined : chunkActions[chunkKey];
    return chunk && id in chunk ? chunk[id] : PENDING_ROW_ACTIONS;
  };
  const moreAvailable = currentData?.next_cursor != null;
  const selectedGroup =
    selectedId === null
      ? null
      : cancelled.some(({ value }) => value.id === selectedId)
        ? CANCELLED_GROUP
        : listMode === "Latest by subject"
          ? groups.find((group) =>
              group.others.some(({ value }) => value.id === selectedId),
            )?.key ?? null
          : null;
  const actionVerbsReady =
    filtered.length > 0 &&
    filtered.every(({ value }) => actionVerbs[value.id] !== undefined);
  const showActionVerb =
    actionVerbsReady &&
    new Set(filtered.map(({ value }) => actionVerbs[value.id])).size > 1;
  const filtersApplied = query !== "" || attention !== "unsnoozed";
  const collapsed =
    !expanded &&
    selectedId === null &&
    !refreshNotice &&
    !workId &&
    !error &&
    !filtersApplied &&
    view === DEFAULT_VIEW &&
    Boolean(currentData) &&
    records.length === 0;

  useEffect(() => {
    if (
      currentData &&
      previous.some(
        (record) =>
          record.value.project.canonical_root !==
            currentData.project.canonical_root ||
          record.value.project.project_id !== currentData.project.project_id,
      )
    ) {
      setPrevious([]);
      setSelectedId(null);
      setCursor(undefined);
    }
  }, [currentData, previous]);

  useLayoutEffect(() => {
    if (selectedGroup !== null) toggleGroup(selectedGroup, true);
  }, [selectedGroup, toggleGroup]);

  useLayoutEffect(() => {
    if (selectedId === null && trigger.current && !restoreViewFocus.current) {
      trigger.current.focus({ preventScroll: true });
    }
  }, [selectedId]);

  useLayoutEffect(() => {
    if (selectedId !== null || !restoreViewFocus.current || isFetching) return;
    const firstRow =
      listRef.current?.querySelector<HTMLButtonElement>(ROW_SELECTOR);
    if (firstRow) firstRow.focus({ preventScroll: true });
    else viewTrigger.current?.focus({ preventScroll: true });
    restoreViewFocus.current = false;
  }, [isFetching, selectedId, view]);

  useEffect(() => {
    const startedAt = refreshStartedAt.current;
    if (!refreshPending || startedAt === null) return;
    if (
      isError &&
      startedTimeStamp !== undefined &&
      startedTimeStamp >= startedAt
    ) {
      refreshStartedAt.current = null;
      setRefreshPending(false);
      return;
    }
    if (
      isFetching ||
      !currentData ||
      fulfilledTimeStamp === undefined ||
      fulfilledTimeStamp < startedAt
    )
      return;
    refreshStartedAt.current = null;
    setRefreshPending(false);
    if (
      selectedId &&
      selectedId !== workId &&
      !currentData.data.some(({ value }) => value.id === selectedId)
    ) {
      if (dismissalPendingId.current === selectedId) {
        refreshStartedAt.current = null;
        setRefreshPending(false);
        refreshFocusPending.current = false;
        return;
      }
      if (dismissedSelectionId.current === selectedId) {
        refreshFocusPending.current = false;
        return;
      }
      if (snoozedSelectionId.current === selectedId) return;
      refreshFocusPending.current = true;
      trigger.current = null;
      setExpanded(true);
      setRefreshNotice(true);
      setSelectedId(null);
    }
  }, [
    currentData,
    fulfilledTimeStamp,
    isError,
    isFetching,
    refreshPending,
    selectedId,
    startedTimeStamp,
    workId,
  ]);

  useLayoutEffect(() => {
    if (!refreshFocusPending.current || selectedId !== null) return;
    const firstRow =
      listRef.current?.querySelector<HTMLButtonElement>(ROW_SELECTOR);
    (firstRow ?? refreshButton.current ?? expandButton.current)?.focus({
      preventScroll: true,
    });
    refreshFocusPending.current = false;
  }, [records, selectedId]);

  useLayoutEffect(() => {
    if (!snoozeFocusPending.current || selectedId !== null || isFetching)
      return;
    const firstRow =
      listRef.current?.querySelector<HTMLButtonElement>(ROW_SELECTOR);
    (firstRow ?? refreshButton.current ?? expandButton.current)?.focus({
      preventScroll: true,
    });
    snoozeFocusPending.current = false;
  }, [isFetching, records, selectedId]);

  useLayoutEffect(() => {
    const pending = dismissalFocusPending.current;
    if (
      !pending ||
      selectedId !== null ||
      records.some(({ value }) => value.id === pending.id)
    )
      return;
    const rows = visibleRows(listRef.current);
    const target = rows.at(pending.index) ?? rows.at(-1);
    (target ?? refreshButton.current ?? expandButton.current)?.focus({
      preventScroll: true,
    });
    dismissalFocusPending.current = null;
    dismissedSelectionId.current = null;
  }, [records, selectedId]);

  const rowIndex = (id: string) =>
    visibleRows(listRef.current).findIndex((row) => row.dataset.deskRow === id);
  const refresh = () => {
    setPrevious([]);
    refreshStartedAt.current = Date.now();
    setRefreshPending(true);
    setRefreshNotice(false);
    setDismissalNotice(null);
    if (cursor === undefined) void refetch();
    else setCursor(undefined);
  };
  const back = () => {
    setSelectedId(null);
    setSelectedOverride(null);
    snoozedSelectionId.current = null;
    if (config.host === "web" && config.engineServed) {
      const url = new URL(window.location.href);
      url.searchParams.delete("buddy_work");
      url.searchParams.delete("buddy_project");
      window.history.replaceState(window.history.state, "", url);
    }
  };
  const updateLink = (item: WorkItem) => {
    if (config.host !== "web" || !config.engineServed) return;
    const url = new URL(window.location.href);
    url.searchParams.set("buddy_work", item.id);
    url.searchParams.set("buddy_project", item.project.project_id);
    url.searchParams.delete("chat");
    window.history.replaceState(window.history.state, "", url);
  };
  const open = (item: WorkItem, source: HTMLButtonElement) => {
    trigger.current = source;
    setDismissalNotice(null);
    setSelectedOverride(null);
    dismissedSelectionId.current = null;
    snoozedSelectionId.current = null;
    setSelectedId(item.id);
    updateLink(item);
  };
  const onDismissPending = (item: WorkItem) => {
    const index = rowIndex(item.id);
    dismissalPendingId.current = item.id;
    dismissalFocusPending.current = { id: item.id, index };
    dismissSnapshotRef.current = item;
    setSelectedOverride(item);
    refreshStartedAt.current = null;
    setRefreshPending(false);
  };
  const onDismissFailure = (item: WorkItem) => {
    if (dismissalPendingId.current !== item.id) return;
    dismissalPendingId.current = null;
    dismissSnapshotRef.current = null;
    dismissalFocusPending.current = null;
    setSelectedOverride(item);
  };
  const onDismiss = (item: WorkItem) => {
    dismissalPendingId.current = null;
    dismissSnapshotRef.current = null;
    dismissalFocusPending.current ??= {
      id: item.id,
      index: rowIndex(item.id),
    };
    dismissedSelectionId.current = item.id;
    trigger.current = null;
    setSelectedOverride(null);
    setRefreshNotice(false);
    setDismissalNotice(
      `Dismissed "${headline(item.title)}". You can find it under Delivered.`,
    );
    setSelectedId(null);
    if (config.host === "web" && config.engineServed) {
      const url = new URL(window.location.href);
      url.searchParams.delete("buddy_work");
      url.searchParams.delete("buddy_project");
      window.history.replaceState(window.history.state, "", url);
    }
  };
  const onSnoozeChange = () => {
    if (!selected) return;
    setSelectedOverride(selected);
    snoozedSelectionId.current = selected.id;
    snoozeFocusPending.current = true;
    void refetch();
  };
  const recordActionVerb = useCallback((id: string, verb: string) => {
    setActionVerbs((current) =>
      current[id] === verb ? current : { ...current, [id]: verb },
    );
  }, []);

  if (collapsed) {
    return (
      <section
        className={styles.collapsed}
        aria-label="Buddy Desk"
        data-detail-open={false}
      >
        <Text
          size="2"
          color="gray"
          className={styles.collapsedText}
          role={dismissalNotice !== null ? "status" : undefined}
        >
          {dismissalNotice ?? "Nothing needs you right now."}
        </Text>
        <Button
          ref={expandButton}
          size="sm"
          variant="ghost"
          leftIcon={Inbox}
          onClick={() => setExpanded(true)}
        >
          Browse saved work
        </Button>
      </section>
    );
  }

  return (
    <Surface
      as="section"
      variant="glass"
      radius="card"
      className={styles.desk}
      aria-label="Buddy Desk"
      data-detail-open={Boolean(selected)}
    >
      <header className={styles.header}>
        <Icon icon={ClipboardList} size="sm" tone="muted" />
        <h2 className={styles.heading}>Buddy Desk</h2>
        <span className={styles.headerSpacer} aria-hidden />
        <Button
          ref={refreshButton}
          size="sm"
          variant="ghost"
          leftIcon={RefreshCw}
          onClick={refresh}
        >
          Refresh work
        </Button>
      </header>
      {refreshNotice && !dismissalNotice && (
        <Text as="p" size="1" color="gray" role="status">
          That item is no longer in this view.
        </Text>
      )}
      {dismissalNotice && (
        <Text as="p" size="1" color="gray" role="status">
          {dismissalNotice}
        </Text>
      )}
      {selectedId === workId && linkedWork.isFetching && (
        <Surface variant="surface-1" className={styles.note} role="status">
          <Text size="2" color="gray">
            Opening that item…
          </Text>
        </Surface>
      )}
      {selectedId === workId && linkedFailure && (
        <Surface variant="surface-1" className={styles.note} role="alert">
          <Text size="2" color="red">
            {projectId
              ? "The linked work is unavailable in this project. No other report was substituted."
              : "This link does not name a project, so the linked work was not opened. No other report was substituted."}
          </Text>
          <div className={styles.noteActions}>
            {projectId && (
              <Button size="sm" onClick={() => void linkedWork.refetch()}>
                Retry linked work
              </Button>
            )}
            <Button size="sm" variant="ghost" onClick={back}>
              Back to work list
            </Button>
          </div>
        </Surface>
      )}
      <div
        className={styles.layout}
        data-split={Boolean(selected) || filtered.length > 0}
      >
        <div className={styles.list} ref={listRef}>
          <nav className={styles.views} aria-label="Work views">
            {views.map((label) => (
              <Button
                key={label}
                size="sm"
                variant={view === label ? "primary" : "ghost"}
                aria-pressed={view === label}
                onClick={(event) => {
                  viewTrigger.current = event.currentTarget;
                  restoreViewFocus.current = selectedId !== null;
                  setView(label);
                  setSelectedId(null);
                  setCursor(undefined);
                  setPrevious([]);
                }}
              >
                {label}
              </Button>
            ))}
          </nav>
          {(records.length > 0 || filtersApplied || expanded) && !error && (
            <div className={styles.filters}>
              {(records.length > 0 || query !== "") && (
                <label className={styles.search}>
                  <Text size="1" color="gray">
                    Search saved work
                  </Text>
                  <input
                    type="search"
                    value={search}
                    maxLength={256}
                    onChange={(event) => {
                      setSearch(event.target.value);
                      setCursor(undefined);
                      setPrevious([]);
                      setSelectedId(null);
                    }}
                  />
                </label>
              )}
              <label className={styles.search}>
                <Text size="1" color="gray">
                  Report attention
                </Text>
                <select
                  value={attention}
                  onChange={(event) => {
                    const value = event.target.value;
                    if (
                      value !== "all" &&
                      value !== "unread" &&
                      value !== "snoozed" &&
                      value !== "unsnoozed"
                    )
                      return;
                    setAttention(value);
                    setCursor(undefined);
                    setPrevious([]);
                    setSelectedId(null);
                  }}
                >
                  <option value="unsnoozed">Active reports</option>
                  <option value="unread">Unread reports</option>
                  <option value="snoozed">Snoozed reports</option>
                  <option value="all">All saved work, including snoozed</option>
                </select>
              </label>
            </div>
          )}
          {!error && filtered.length > 0 && (
            <div className={styles.listTools}>
              {primary.length > 0 && (
                <div
                  className={styles.modes}
                  role="group"
                  aria-label="List layout"
                >
                  {listModes.map((label) => (
                    <Button
                      key={label}
                      size="sm"
                      variant={listMode === label ? "soft" : "ghost"}
                      aria-pressed={listMode === label}
                      onClick={() => setListMode(label)}
                    >
                      {label}
                    </Button>
                  ))}
                </div>
              )}
              <Text
                size="1"
                color="gray"
                className={styles.count}
                data-testid="buddy-desk-count"
              >
                {[
                  `${plural(filtered.length, "record")} loaded`,
                  listMode === "Latest by subject" && primary.length > 0
                    ? `${plural(groups.length, "subject")}`
                    : null,
                  cancelled.length > 0 ? `${cancelled.length} cancelled` : null,
                  moreAvailable ? "more saved work not loaded yet" : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </Text>
              {listMode === "Latest by subject" &&
                repeatedSubjects.length > 0 && (
                  <Text size="1" color="gray" className={styles.count}>
                    Records are grouped only when they share the same work kind
                    and saved subject. Grouping covers loaded records only.
                  </Text>
                )}
            </div>
          )}
          {isFetching && (
            <Text
              as="p"
              size="1"
              color="gray"
              role="status"
              className={styles.inlineStatus}
            >
              Loading work…
            </Text>
          )}
          {error ? (
            <Surface variant="surface-1" className={styles.note} role="alert">
              <Text size="2" color="red">
                Buddy could not reach your saved work.
              </Text>
              <Button size="sm" onClick={refresh}>
                Retry work
              </Button>
            </Surface>
          ) : (
            <>
              {currentData && primary.length === 0 && (
                <Text
                  as="p"
                  size="2"
                  color="gray"
                  className={styles.inlineStatus}
                >
                  {cancelled.length > 0
                    ? "No delivered results among the loaded records. Cancelled work is listed separately."
                    : filtersApplied
                      ? "Nothing here matches those filters."
                      : emptyView[view]}
                </Text>
              )}
              {actionChunks.map((chunk) => (
                <ActionBatchSubscriber
                  key={chunk.key}
                  chunkKey={chunk.key}
                  chunk={chunk.items}
                  endpointIdentity={endpointIdentity}
                  onResult={recordChunkActions}
                />
              ))}
              {(() => {
                const row = (record: OperatorRecord<WorkItem>) => (
                  <WorkRow
                    key={record.value.id}
                    item={record.value}
                    endpointIdentity={endpointIdentity}
                    showState={distinctStates.size > 1}
                    showActionVerb={showActionVerb}
                    rowActions={rowActionsFor(record.value.id)}
                    selected={selectedId === record.value.id}
                    onOpen={open}
                    onActionVerb={recordActionVerb}
                  />
                );
                return (
                  <div
                    className={styles.scroller}
                    data-testid="buddy-desk-scroller"
                  >
                    {primary.length > 0 && listMode === "Run history" && (
                      <ul className={styles.items}>{primary.map(row)}</ul>
                    )}
                    {primary.length > 0 && listMode === "Latest by subject" && (
                      <ul className={styles.items}>
                        {groups.map((group) => (
                          <li key={group.key} className={styles.group}>
                            <ul className={styles.items}>{row(group.head)}</ul>
                            {group.others.length > 0 && (
                              <details
                                className={styles.history}
                                open={openGroups.has(group.key)}
                                onToggle={(event) =>
                                  toggleGroup(
                                    group.key,
                                    event.currentTarget.open,
                                  )
                                }
                              >
                                <summary className={styles.historySummary}>
                                  {`${plural(
                                    group.others.length,
                                    "other loaded record",
                                  )} for this subject (${
                                    group.others.length + 1
                                  } loaded)`}
                                </summary>
                                <ul className={styles.items}>
                                  {group.others.map(row)}
                                </ul>
                              </details>
                            )}
                          </li>
                        ))}
                      </ul>
                    )}
                    {cancelled.length > 0 && primary.length === 0 && (
                      <section
                        className={styles.cancelled}
                        aria-label="Cancelled work"
                      >
                        <Text size="1" color="gray">
                          {`Cancelled (${cancelled.length} loaded) — kept for history, not delivered results`}
                        </Text>
                        <ul className={styles.items}>{cancelled.map(row)}</ul>
                      </section>
                    )}
                    {cancelled.length > 0 && primary.length > 0 && (
                      <details
                        className={styles.cancelled}
                        aria-label="Cancelled work"
                        open={openGroups.has(CANCELLED_GROUP)}
                        onToggle={(event) =>
                          toggleGroup(CANCELLED_GROUP, event.currentTarget.open)
                        }
                      >
                        <summary className={styles.historySummary}>
                          {`Cancelled (${cancelled.length} loaded) — kept for history, not delivered results`}
                        </summary>
                        <ul className={styles.items}>{cancelled.map(row)}</ul>
                      </details>
                    )}
                  </div>
                );
              })()}
              {currentData?.next_cursor != null && (
                <Button
                  size="sm"
                  disabled={isFetching}
                  onClick={() => {
                    setPrevious(records);
                    setCursor(currentData.next_cursor ?? undefined);
                  }}
                >
                  Load more work
                </Button>
              )}
            </>
          )}
        </div>
        {selected && !(selectedId === workId ? linkedFailure : error) ? (
          <div className={styles.detail}>
            <BuddyDeskDetail
              key={`${selected.id}:${selected.current_artifact_revision ?? 0}`}
              item={selected}
              endpointIdentity={endpointIdentity}
              onBack={back}
              onSnoozeChange={onSnoozeChange}
              onDismiss={onDismiss}
              onDismissPending={onDismissPending}
              onDismissFailure={onDismissFailure}
            />
          </div>
        ) : (
          filtered.length > 0 && (
            <div className={styles.placeholder}>
              <Icon icon={FileSearch} size="md" tone="muted" />
              <Text size="2" color="gray">
                Choose a report to read what Buddy found.
              </Text>
            </div>
          )
        )}
      </div>
    </Surface>
  );
}

export function BuddyDesk({
  workId,
  projectId,
}: {
  workId?: string;
  projectId?: string;
}) {
  const identity = useAppSelector(buddyOperatorIdentity);
  return (
    <DeskContent
      key={`${identity}:${projectId ?? ""}:${workId ?? ""}`}
      endpointIdentity={identity}
      workId={workId}
      projectId={projectId}
    />
  );
}
