import type { Ref } from "react";
import { Inbox } from "lucide-react";
import type { OperatorRecord } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import {
  EMPTY_VIEW_COPY,
  VIEW_DEFAULT_STATE,
  plural,
  type DeskAttention,
  type DeskListMode,
} from "./BuddyDeskModel";
import type { BuddyReportsView } from "./sections/buddyReportsLink";
import { BuddyEmptyState, BuddyErrorCallout, BuddyLoading } from "./ui";
import { DeskRow } from "./BuddyDeskRow";
import { BuddyDeskList } from "./BuddyDeskList";
import { BuddyDeskActionBatches } from "./BuddyDeskActionBatches";
import {
  BuddyDeskFilters,
  BuddyDeskListTools,
  BuddyDeskViews,
} from "./BuddyDeskToolbar";
import type { DeskWorkList } from "./hooks/useDeskWorkList";
import type { DeskRowActions } from "./hooks/useDeskRowActions";
import styles from "./BuddyDesk.module.css";

function deskCountText(workList: DeskWorkList, listMode: DeskListMode): string {
  const { filtered, primary, groups, cancelled, moreAvailable } = workList;
  return [
    plural(filtered.length, "report"),
    listMode === "latest" && primary.length > 0
      ? plural(groups.length, "subject")
      : null,
    cancelled.length > 0 ? `${cancelled.length} cancelled` : null,
    moreAvailable ? "more available" : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

function emptyTitle(workList: DeskWorkList, view: BuddyReportsView): string {
  if (workList.cancelled.length > 0)
    return "No delivered reports here. Cancelled reports are listed below.";
  if (workList.filtersApplied) return "No reports match these filters.";
  return EMPTY_VIEW_COPY[view];
}

export function BuddyDeskListPane({
  workList,
  rowActions,
  view,
  endpointIdentity,
  selectedId,
  expanded,
  listMode,
  openGroups,
  listRef,
  viewsRef,
  onChangeView,
  onSearch,
  onAttention,
  onListMode,
  onToggleGroup,
  onOpen,
  onRetry,
}: {
  workList: DeskWorkList;
  rowActions: DeskRowActions;
  view: BuddyReportsView;
  endpointIdentity: string;
  selectedId: string | null;
  expanded: boolean;
  listMode: DeskListMode;
  openGroups: ReadonlySet<string>;
  listRef: Ref<HTMLDivElement>;
  viewsRef: Ref<HTMLDivElement>;
  onChangeView: (view: BuddyReportsView) => void;
  onSearch: (value: string) => void;
  onAttention: (value: DeskAttention) => void;
  onListMode: (mode: DeskListMode) => void;
  onToggleGroup: (key: string, open: boolean) => void;
  onOpen: (item: WorkItem, source: HTMLElement) => void;
  onRetry: () => void;
}) {
  const { list, records, filtered, primary, cancelled, groups } = workList;
  const { error, currentData } = list;
  const defaultState = VIEW_DEFAULT_STATE[view];
  const renderRow = (record: OperatorRecord<WorkItem>, bare = false) => (
    <DeskRow
      key={record.value.id}
      item={record.value}
      endpointIdentity={endpointIdentity}
      defaultState={defaultState}
      showActionVerb={rowActions.showActionVerb}
      rowActions={rowActions.rowActionsFor(record.value.id)}
      selected={selectedId === record.value.id}
      unread={workList.isUnread(record.value)}
      bare={bare}
      onOpen={onOpen}
      onActionVerb={rowActions.recordActionVerb}
    />
  );
  const showFilters =
    (records.length > 0 || workList.filtersApplied || expanded) && !error;
  return (
    <div className={styles.list} ref={listRef}>
      <BuddyDeskViews view={view} onChange={onChangeView} viewsRef={viewsRef} />
      {showFilters && (
        <BuddyDeskFilters
          search={workList.search}
          showSearch={records.length > 0 || workList.query !== ""}
          attention={workList.attention}
          onSearch={onSearch}
          onAttention={onAttention}
        />
      )}
      {!error && filtered.length > 0 && (
        <BuddyDeskListTools
          listMode={listMode}
          showModes={primary.length > 0}
          count={deskCountText(workList, listMode)}
          grouped={
            listMode === "latest" &&
            groups.some((group) => group.others.length > 0)
          }
          onListMode={onListMode}
        />
      )}
      {workList.initialLoading && (
        <BuddyLoading variant="rows" count={3} label="Loading reports" />
      )}
      {!error && workList.unreadError !== undefined && (
        <BuddyErrorCallout
          compact
          thing="unread markers"
          error={workList.unreadError}
          onRetry={workList.refetchUnread}
          retryLabel="Retry unread markers"
        />
      )}
      {error ? (
        <BuddyErrorCallout
          thing="reports"
          error={error}
          onRetry={onRetry}
          retryLabel="Retry reports"
        />
      ) : (
        <>
          {currentData && primary.length === 0 && (
            <BuddyEmptyState
              compact
              icon={Inbox}
              title={emptyTitle(workList, view)}
            />
          )}
          <BuddyDeskActionBatches
            chunks={rowActions.actionChunks}
            endpointIdentity={endpointIdentity}
            onResult={rowActions.recordChunkActions}
          />
          <BuddyDeskList
            listMode={listMode}
            primary={primary}
            cancelled={cancelled}
            groups={groups}
            openGroups={openGroups}
            onToggleGroup={onToggleGroup}
            renderRow={renderRow}
          />
        </>
      )}
    </div>
  );
}
