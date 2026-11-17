import { forwardRef, useEffect, useState } from "react";
import {
  ClipboardList,
  FileText,
  RefreshCw,
  type LucideProps,
} from "lucide-react";
import { Button } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import { DEFAULT_DESK_VIEW } from "./BuddyDeskModel";
import {
  BUDDY_REPORTS_VIEW_KEY,
  type BuddyReportsView,
} from "./sections/buddyReportsLink";
import {
  BUDDY_TERMS,
  BuddyEmptyState,
  BuddyLoading,
  BuddySection,
  useBuddyPersistentState,
} from "./ui";
import { BuddyDeskDetail } from "./BuddyDeskDetail";
import { BuddyDeskCollapsed } from "./BuddyDeskCollapsed";
import { BuddyDeskNotices } from "./BuddyDeskNotices";
import { BuddyDeskListPane } from "./BuddyDeskListPane";
import { useDeskWorkList } from "./hooks/useDeskWorkList";
import { useDeskLinkedWork } from "./hooks/useDeskLinkedWork";
import { useDeskRowActions } from "./hooks/useDeskRowActions";
import { useDeskSelection } from "./hooks/useDeskSelection";
import { useDeskGroups } from "./hooks/useDeskGroups";
import {
  useDeskFocusRefs,
  useDeskFocusRestore,
} from "./hooks/useDeskFocusRestore";
import { useDeskDetailHandlers } from "./hooks/useDeskDetailHandlers";
import styles from "./BuddyDesk.module.css";

const SpinningRefresh = forwardRef<SVGSVGElement, LucideProps>(
  function SpinningRefresh({ className, ...props }, ref) {
    return (
      <RefreshCw
        {...props}
        ref={ref}
        className={className ? `${className} rf-spin` : "rf-spin"}
      />
    );
  },
);

function useDeskController({
  endpointIdentity,
  workId,
  projectId,
}: {
  endpointIdentity: string;
  workId?: string;
  projectId?: string;
}) {
  const [view, setStoredView] = useBuddyPersistentState<BuddyReportsView>(
    BUDDY_REPORTS_VIEW_KEY,
    DEFAULT_DESK_VIEW,
  );
  const selection = useDeskSelection({ endpointIdentity, workId, view });
  const { selectedId } = selection;
  const focus = useDeskFocusRefs();
  const [expanded, setExpanded] = useState(false);
  const [refreshNotice, setRefreshNotice] = useState(false);
  const [dismissalNotice, setDismissalNotice] = useState<string | null>(null);
  const linked = useDeskLinkedWork({ endpointIdentity, workId, projectId });
  const workList = useDeskWorkList({
    endpointIdentity,
    view,
    setSelectedId: selection.setSelectedId,
    onFiltersApplied: () => resetList(),
  });
  const { list, records } = workList;
  const selected = selection.resolveSelected(records, linked.linkedValue);
  const restoredGone =
    selection.restoredFailed && selected === undefined && !list.isFetching;
  const [restoreNotice, setRestoreNotice] = useState<string | null>(null);
  const restoreError = Boolean(selection.restoredWork.error);
  useEffect(() => {
    if (!restoredGone) return;
    selection.clearSelection();
    setExpanded(true);
    setRestoreNotice(
      restoreError
        ? "Couldn't reopen the last report you were reading."
        : "The last report you were reading is no longer in this view.",
    );
  }, [restoreError, restoredGone, selection]);
  const rowActions = useDeskRowActions({
    filtered: workList.filtered,
    endpointIdentity,
  });
  const groups = useDeskGroups({
    selectedId,
    groups: workList.groups,
    cancelled: workList.cancelled,
  });
  const refreshState = useDeskFocusRestore({
    focus,
    list,
    records,
    selectedId,
    view,
    workId,
    onSelectionGone: () => {
      setExpanded(true);
      setRefreshNotice(true);
      selection.clearSelection();
    },
  });
  const handlers = useDeskDetailHandlers({
    focus,
    selection,
    selected,
    setRefreshNotice,
    setDismissalNotice,
    cancelRefresh: refreshState.cancelRefresh,
    refetchList: () => void list.refetch(),
    refetchUnread: workList.refetchUnread,
  });
  function resetList() {
    workList.resetPaging();
    selection.clearSelection();
  }
  return {
    view,
    selection,
    selected,
    restoredGone,
    focus,
    expanded,
    setExpanded,
    refreshNotice,
    dismissalNotice,
    restoreNotice,
    refreshPending: refreshState.refreshPending,
    linked,
    workList,
    rowActions,
    groups,
    handlers,
    refresh: () => {
      refreshState.startRefresh();
      setRefreshNotice(false);
      setDismissalNotice(null);
      setRestoreNotice(null);
      workList.reload();
    },
    open: (item: WorkItem, source: HTMLElement) => {
      focus.trigger.current = source;
      setDismissalNotice(null);
      setRestoreNotice(null);
      focus.dismissedSelectionId.current = null;
      focus.snoozedSelectionId.current = null;
      selection.select(item);
    },
    changeView: (next: BuddyReportsView) => {
      focus.restoreViewFocus.current = selectedId !== null;
      setStoredView(next);
      resetList();
    },
    resetList,
  };
}

type DeskController = ReturnType<typeof useDeskController>;

function isCollapsed(desk: DeskController, workId?: string): boolean {
  const { list, filtersApplied, records } = desk.workList;
  return (
    !desk.expanded &&
    desk.selection.selectedId === null &&
    !desk.refreshNotice &&
    !desk.restoreNotice &&
    !workId &&
    !list.error &&
    !filtersApplied &&
    desk.view === DEFAULT_DESK_VIEW &&
    Boolean(list.currentData) &&
    records.length === 0
  );
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
  const desk = useDeskController({ endpointIdentity, workId, projectId });
  const { selection, selected, focus, workList, linked, handlers } = desk;
  const { selectedId } = selection;
  const { list } = workList;

  if (isCollapsed(desk, workId)) {
    return (
      <BuddyDeskCollapsed
        dismissalNotice={desk.dismissalNotice}
        expandButton={focus.expandButton}
        onExpand={() => desk.setExpanded(true)}
      />
    );
  }

  const linkedSelected = selectedId === workId;
  const showDetail =
    selected !== undefined &&
    !(linkedSelected ? linked.linkedFailure : list.error);
  const detailPending =
    selectedId !== null &&
    !linkedSelected &&
    selected === undefined &&
    !list.error &&
    !desk.restoredGone;
  const showHint =
    selectedId === null && !list.error && workList.filtered.length > 0;
  return (
    <BuddySection
      title={BUDDY_TERMS.reports}
      icon={ClipboardList}
      className={styles.desk}
      bodyClassName={styles.deskBody}
      aria-label={BUDDY_TERMS.reports}
      testId="buddy-desk"
      action={
        <Button
          ref={focus.refreshButton}
          size="sm"
          variant="ghost"
          leftIcon={desk.refreshPending ? SpinningRefresh : RefreshCw}
          aria-busy={desk.refreshPending || undefined}
          className={desk.refreshPending ? styles.refreshBusy : undefined}
          onClick={desk.refresh}
        >
          Refresh reports
        </Button>
      }
      footer={
        workList.moreAvailable && !list.error ? (
          <Button
            size="sm"
            variant="soft"
            disabled={list.isFetching}
            onClick={workList.loadMore}
          >
            Load more reports
          </Button>
        ) : undefined
      }
    >
      {desk.refreshPending && (
        <span role="status" className={styles.srOnly}>
          Refreshing reports…
        </span>
      )}
      <BuddyDeskNotices
        notice={
          desk.dismissalNotice ??
          desk.restoreNotice ??
          (desk.refreshNotice ? "That report is no longer in this view." : null)
        }
        linkedLoading={linkedSelected && linked.linkedWork.isFetching}
        linkedFailure={linkedSelected && linked.linkedFailure}
        linkedError={linked.linkedWork.error}
        projectId={projectId}
        onRetryLinked={() => void linked.linkedWork.refetch()}
      />
      <div
        className={styles.layout}
        data-detail-open={Boolean(selected)}
        data-detail-visible={showDetail || detailPending}
        data-testid="buddy-desk-layout"
      >
        <BuddyDeskListPane
          workList={workList}
          rowActions={desk.rowActions}
          view={desk.view}
          endpointIdentity={endpointIdentity}
          selectedId={selectedId}
          expanded={desk.expanded}
          listMode={desk.groups.listMode}
          openGroups={desk.groups.openGroups}
          listRef={focus.listRef}
          viewsRef={focus.viewTrigger}
          onChangeView={desk.changeView}
          onSearch={(value) => {
            workList.setSearch(value);
          }}
          onAttention={(value) => {
            workList.setAttention(value);
            desk.resetList();
          }}
          onListMode={desk.groups.setListMode}
          onToggleGroup={desk.groups.toggleGroup}
          onOpen={desk.open}
          onRetry={desk.refresh}
        />
        {showHint && (
          <div
            className={styles.detailHint}
            data-testid="buddy-desk-detail-hint"
          >
            <BuddyEmptyState
              compact
              icon={FileText}
              title="Pick a report to read it here"
              hint="Findings, write-up and evidence open on this side."
            />
          </div>
        )}
        {detailPending && (
          <div
            className={styles.detail}
            data-testid="buddy-desk-detail-loading"
          >
            <BuddyLoading
              variant="cards"
              count={1}
              label="Opening the report"
            />
          </div>
        )}
        {showDetail && (
          <div className={styles.detail}>
            <BuddyDeskDetail
              key={`${selected.id}:${selected.current_artifact_revision ?? 0}`}
              item={selected}
              endpointIdentity={endpointIdentity}
              onBack={handlers.back}
              onSnoozeChange={handlers.onSnoozeChange}
              onDismiss={handlers.onDismiss}
              onDismissPending={handlers.onDismissPending}
              onDismissFailure={handlers.onDismissFailure}
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
