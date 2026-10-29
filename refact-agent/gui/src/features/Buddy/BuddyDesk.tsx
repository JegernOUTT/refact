import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ClipboardList, FileText, Inbox, RefreshCw } from "lucide-react";
import { Button } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import {
  useGetBuddyDeskActionsBatchQuery,
  useGetBuddyOperatorWorkQuery,
  useGetBuddyOperatorWorkItemQuery,
  buddyOperatorIdentity,
  type OperatorRecord,
} from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import { useConfig } from "../../hooks/useConfig";
import { headline } from "./buddyUtils";
import {
  CANCELLED_GROUP,
  DEFAULT_DESK_VIEW,
  EMPTY_VIEW_COPY,
  PENDING_ROW_ACTIONS,
  ROW_SELECTOR,
  VIEW_DEFAULT_STATE,
  VIEW_FOR_STATE,
  chunkWork,
  groupDeskRecords,
  plural,
  sameChunkActions,
  type ChunkActions,
  type DeskAttention,
  type DeskListMode,
  type RowActions,
} from "./BuddyDeskModel";
import {
  BUDDY_REPORTS_SELECTION_KEY,
  BUDDY_REPORTS_VIEW_KEY,
  type BuddyReportsSelection,
  type BuddyReportsView,
} from "./sections/buddyReportsLink";
import {
  BUDDY_TERMS,
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyLoading,
  BuddySection,
  useBuddyPersistentState,
} from "./ui";
import { BuddyDeskDetail } from "./BuddyDeskDetail";
import { DeskRow } from "./BuddyDeskRow";
import { BuddyDeskList } from "./BuddyDeskList";
import {
  BuddyDeskFilters,
  BuddyDeskListTools,
  BuddyDeskViews,
} from "./BuddyDeskToolbar";
import styles from "./BuddyDesk.module.css";

function visibleRows(list: HTMLElement | null): HTMLButtonElement[] {
  return [
    ...(list?.querySelectorAll<HTMLButtonElement>(ROW_SELECTOR) ?? []),
  ].filter((row) => !row.closest("[hidden]"));
}

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
  const [view, setStoredView] = useBuddyPersistentState<BuddyReportsView>(
    BUDDY_REPORTS_VIEW_KEY,
    DEFAULT_DESK_VIEW,
  );
  const [storedSelection, setStoredSelection] =
    useBuddyPersistentState<BuddyReportsSelection | null>(
      BUDDY_REPORTS_SELECTION_KEY,
      null,
    );
  const [restored] = useState(() =>
    !workId &&
    storedSelection?.endpointIdentity === endpointIdentity &&
    storedSelection.view === view
      ? storedSelection
      : null,
  );
  const [listMode, setListMode] = useState<DeskListMode>("latest");
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
  const [selectedId, setSelectedId] = useState<string | null>(
    workId ?? restored?.workId ?? null,
  );
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
  const [attention, setAttention] = useState<DeskAttention>("unsnoozed");
  const restoredWork = useGetBuddyOperatorWorkItemQuery(
    {
      id: restored?.workId ?? "",
      project_id: restored?.projectId,
      endpoint_identity: endpointIdentity,
    },
    { skip: !restored || selectedId !== restored.workId },
  );
  const listRef = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLElement | null>(null);
  const viewTrigger = useRef<HTMLDivElement | null>(null);
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
      view,
      search: search.trim(),
      attention,
    },
    { refetchOnMountOrArgChange: true },
  );
  const unreadSkipped = attention === "unread" || !currentData?.data.length;
  const unreadQuery = useGetBuddyOperatorWorkQuery(
    {
      limit: 200,
      endpoint_identity: endpointIdentity,
      view,
      search: search.trim(),
      attention: "unread",
    },
    { skip: unreadSkipped },
  );
  const unreadIds = useMemo(
    () => new Set(unreadQuery.currentData?.data.map(({ value }) => value.id)),
    [unreadQuery.currentData],
  );
  const isUnread = (item: WorkItem) =>
    attention === "unread" || unreadIds.has(item.id);
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
          : undefined) ??
        (restored?.workId === selectedId &&
        restoredWork.currentData?.data.value.id === selectedId &&
        restoredWork.currentData.data.value.project.project_id ===
          restored.projectId
          ? restoredWork.currentData.data.value
          : undefined);
  const query = search.trim().toLocaleLowerCase();
  const filtered = records.filter(
    ({ value }) => VIEW_FOR_STATE[value.state] === view,
  );
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
        : listMode === "latest"
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
    view === DEFAULT_DESK_VIEW &&
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
    else
      viewTrigger.current
        ?.querySelector<HTMLInputElement>("input:checked")
        ?.focus({ preventScroll: true });
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
      setStoredSelection(null);
    }
  }, [
    currentData,
    fulfilledTimeStamp,
    isError,
    isFetching,
    refreshPending,
    setStoredSelection,
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
    setStoredSelection(null);
    if (!unreadSkipped) void unreadQuery.refetch();
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
  const open = (item: WorkItem, source: HTMLElement) => {
    trigger.current = source;
    setDismissalNotice(null);
    setSelectedOverride(null);
    dismissedSelectionId.current = null;
    snoozedSelectionId.current = null;
    setSelectedId(item.id);
    setStoredSelection({
      workId: item.id,
      projectId: item.project.project_id,
      endpointIdentity,
      view,
    });
    updateLink(item);
  };
  const resetList = () => {
    setCursor(undefined);
    setPrevious([]);
    setSelectedId(null);
    setStoredSelection(null);
  };
  const changeView = (next: BuddyReportsView) => {
    restoreViewFocus.current = selectedId !== null;
    setStoredView(next);
    resetList();
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
    setStoredSelection(null);
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

  const countText = [
    plural(filtered.length, "report"),
    listMode === "latest" && primary.length > 0
      ? plural(groups.length, "subject")
      : null,
    cancelled.length > 0 ? `${cancelled.length} cancelled` : null,
    moreAvailable ? "more available" : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const defaultState = VIEW_DEFAULT_STATE[view];
  const renderRow = (record: OperatorRecord<WorkItem>) => (
    <DeskRow
      key={record.value.id}
      item={record.value}
      endpointIdentity={endpointIdentity}
      defaultState={defaultState}
      showActionVerb={showActionVerb}
      rowActions={rowActionsFor(record.value.id)}
      selected={selectedId === record.value.id}
      unread={isUnread(record.value)}
      onOpen={open}
      onActionVerb={recordActionVerb}
    />
  );
  const refreshAction = (
    <Button
      ref={refreshButton}
      size="sm"
      variant="ghost"
      leftIcon={RefreshCw}
      onClick={refresh}
    >
      Refresh reports
    </Button>
  );

  if (collapsed) {
    return (
      <BuddySection
        title={BUDDY_TERMS.reports}
        icon={ClipboardList}
        className={styles.desk}
        aria-label={BUDDY_TERMS.reports}
        testId="buddy-desk"
      >
        <BuddyEmptyState
          compact
          icon={Inbox}
          title={
            dismissalNotice ? (
              <span role="status">{dismissalNotice}</span>
            ) : (
              EMPTY_VIEW_COPY.needs_you
            )
          }
          action={
            <Button
              ref={expandButton}
              size="sm"
              variant="ghost"
              onClick={() => setExpanded(true)}
            >
              Browse reports
            </Button>
          }
        />
      </BuddySection>
    );
  }

  const detailOpen = Boolean(selected);
  const showDetail =
    selected !== undefined && !(selectedId === workId ? linkedFailure : error);
  return (
    <BuddySection
      title={BUDDY_TERMS.reports}
      icon={ClipboardList}
      className={styles.desk}
      bodyClassName={styles.deskBody}
      aria-label={BUDDY_TERMS.reports}
      testId="buddy-desk"
      action={refreshAction}
      footer={
        moreAvailable && !error ? (
          <Button
            size="sm"
            variant="soft"
            disabled={isFetching}
            onClick={() => {
              setPrevious(records);
              setCursor(currentData.next_cursor ?? undefined);
            }}
          >
            Load more reports
          </Button>
        ) : undefined
      }
    >
      {(refreshNotice || dismissalNotice) && (
        <p role="status" className={styles.notice}>
          {dismissalNotice ?? "That report is no longer in this view."}
        </p>
      )}
      {selectedId === workId && linkedWork.isFetching && (
        <BuddyLoading variant="rows" count={1} label="Opening that report" />
      )}
      {selectedId === workId && linkedFailure && (
        <BuddyErrorCallout
          thing="that report"
          title={
            projectId
              ? "Couldn't open that report in this project. No other report was opened."
              : "This link doesn't name a project, so the report wasn't opened."
          }
          error={linkedWork.error}
          onRetry={projectId ? () => void linkedWork.refetch() : undefined}
        />
      )}
      <div
        className={styles.layout}
        data-detail-open={detailOpen}
        data-detail-visible={Boolean(showDetail)}
        data-testid="buddy-desk-layout"
      >
        <div className={styles.list} ref={listRef}>
          <BuddyDeskViews
            view={view}
            onChange={changeView}
            viewsRef={viewTrigger}
          />
          {(records.length > 0 || filtersApplied || expanded) && !error && (
            <BuddyDeskFilters
              search={search}
              showSearch={records.length > 0 || query !== ""}
              attention={attention}
              onSearch={(value) => {
                setSearch(value);
                resetList();
              }}
              onAttention={(value) => {
                setAttention(value);
                resetList();
              }}
            />
          )}
          {!error && filtered.length > 0 && (
            <BuddyDeskListTools
              listMode={listMode}
              showModes={primary.length > 0}
              count={countText}
              grouped={listMode === "latest" && repeatedSubjects.length > 0}
              onListMode={setListMode}
            />
          )}
          {isFetching && (
            <BuddyLoading variant="rows" count={3} label="Loading reports" />
          )}
          {error ? (
            <BuddyErrorCallout
              thing="reports"
              error={error}
              onRetry={refresh}
              retryLabel="Retry reports"
            />
          ) : (
            <>
              {currentData && primary.length === 0 && (
                <BuddyEmptyState
                  compact
                  icon={Inbox}
                  title={
                    cancelled.length > 0
                      ? "No delivered reports here. Cancelled reports are listed below."
                      : filtersApplied
                        ? "No reports match these filters."
                        : EMPTY_VIEW_COPY[view]
                  }
                />
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
              <BuddyDeskList
                listMode={listMode}
                primary={primary}
                cancelled={cancelled}
                groups={groups}
                openGroups={openGroups}
                onToggleGroup={toggleGroup}
                renderRow={renderRow}
              />
            </>
          )}
        </div>
        {selectedId === null && (
          <div
            className={styles.detailHint}
            data-testid="buddy-desk-detail-hint"
          >
            <BuddyEmptyState
              icon={FileText}
              title="Pick a report to read it here"
              hint="Findings, write-up and evidence open on this side."
            />
          </div>
        )}
        {showDetail && (
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
        )}
      </div>
    </BuddySection>
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
