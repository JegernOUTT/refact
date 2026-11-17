import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type MutableRefObject,
} from "react";
import type {
  OperatorPage,
  OperatorRecord,
} from "../../../services/refact/buddyOperator";
import type { WorkItem } from "../operatorTypes";
import { ROW_SELECTOR } from "../BuddyDeskModel";
import type { BuddyReportsView } from "../sections/buddyReportsLink";

export function visibleRows(list: HTMLElement | null): HTMLButtonElement[] {
  return [
    ...(list?.querySelectorAll<HTMLButtonElement>(ROW_SELECTOR) ?? []),
  ].filter((row) => !row.closest("[hidden]"));
}

function box<T>(value: T): MutableRefObject<T> {
  return { current: value };
}

function createDeskFocusRefs() {
  return {
    listRef: box<HTMLDivElement | null>(null),
    trigger: box<HTMLElement | null>(null),
    viewTrigger: box<HTMLDivElement | null>(null),
    restoreViewFocus: box(false),
    expandButton: box<HTMLButtonElement | null>(null),
    refreshButton: box<HTMLButtonElement | null>(null),
    refreshStartedAt: box<number | null>(null),
    refreshFocusPending: box(false),
    snoozeFocusPending: box(false),
    dismissalPendingId: box<string | null>(null),
    dismissalFocusPending: box<{ id: string; index: number } | null>(null),
    dismissedSelectionId: box<string | null>(null),
    snoozedSelectionId: box<string | null>(null),
  };
}

export type DeskFocusRefs = ReturnType<typeof createDeskFocusRefs>;

export function useDeskFocusRefs(): DeskFocusRefs {
  const [refs] = useState(createDeskFocusRefs);
  return refs;
}

type ListQueryState = {
  currentData?: OperatorPage<OperatorRecord<WorkItem>[]>;
  isFetching: boolean;
  isError: boolean;
  startedTimeStamp?: number;
  fulfilledTimeStamp?: number;
};

function focusFirst(
  focus: DeskFocusRefs,
  candidate: HTMLElement | null | undefined,
) {
  (
    candidate ??
    focus.refreshButton.current ??
    focus.expandButton.current
  )?.focus({ preventScroll: true });
}

function firstRow(focus: DeskFocusRefs) {
  return focus.listRef.current?.querySelector<HTMLButtonElement>(ROW_SELECTOR);
}

function refreshSettled(
  list: ListQueryState,
  startedAt: number,
): "failed" | "pending" | "done" {
  if (
    list.isError &&
    list.startedTimeStamp !== undefined &&
    list.startedTimeStamp >= startedAt
  )
    return "failed";
  if (
    list.isFetching ||
    !list.currentData ||
    list.fulfilledTimeStamp === undefined ||
    list.fulfilledTimeStamp < startedAt
  )
    return "pending";
  return "done";
}

function useRefreshReconcile({
  focus,
  list,
  selectedId,
  workId,
  onSelectionGone,
}: {
  focus: DeskFocusRefs;
  list: ListQueryState;
  selectedId: string | null;
  workId?: string;
  onSelectionGone: () => void;
}) {
  const [refreshPending, setRefreshPending] = useState(false);
  const goneRef = useRef(onSelectionGone);
  goneRef.current = onSelectionGone;
  const { currentData, fulfilledTimeStamp, isError, isFetching } = list;
  const { startedTimeStamp } = list;
  useEffect(() => {
    const startedAt = focus.refreshStartedAt.current;
    if (!refreshPending || startedAt === null) return;
    const settled = refreshSettled(
      {
        currentData,
        fulfilledTimeStamp,
        isError,
        isFetching,
        startedTimeStamp,
      },
      startedAt,
    );
    if (settled === "pending") return;
    focus.refreshStartedAt.current = null;
    setRefreshPending(false);
    if (
      settled === "failed" ||
      !currentData ||
      !selectedId ||
      selectedId === workId ||
      currentData.data.some(({ value }) => value.id === selectedId)
    )
      return;
    if (focus.dismissalPendingId.current === selectedId) {
      focus.refreshFocusPending.current = false;
      return;
    }
    if (focus.dismissedSelectionId.current === selectedId) {
      focus.refreshFocusPending.current = false;
      return;
    }
    if (focus.snoozedSelectionId.current === selectedId) return;
    focus.refreshFocusPending.current = true;
    focus.trigger.current = null;
    goneRef.current();
  }, [
    currentData,
    focus,
    fulfilledTimeStamp,
    isError,
    isFetching,
    refreshPending,
    selectedId,
    startedTimeStamp,
    workId,
  ]);
  return {
    refreshPending,
    startRefresh: () => {
      focus.refreshStartedAt.current = Date.now();
      setRefreshPending(true);
    },
    cancelRefresh: () => {
      focus.refreshStartedAt.current = null;
      setRefreshPending(false);
    },
  };
}

export function useDeskFocusRestore({
  focus,
  list,
  records,
  selectedId,
  view,
  workId,
  onSelectionGone,
}: {
  focus: DeskFocusRefs;
  list: ListQueryState;
  records: OperatorRecord<WorkItem>[];
  selectedId: string | null;
  view: BuddyReportsView;
  workId?: string;
  onSelectionGone: () => void;
}) {
  const { isFetching } = list;

  useLayoutEffect(() => {
    if (
      selectedId === null &&
      focus.trigger.current &&
      !focus.restoreViewFocus.current
    ) {
      focus.trigger.current.focus({ preventScroll: true });
    }
  }, [focus, selectedId]);

  useLayoutEffect(() => {
    if (selectedId !== null || !focus.restoreViewFocus.current || isFetching)
      return;
    const row = firstRow(focus);
    if (row) row.focus({ preventScroll: true });
    else
      focus.viewTrigger.current
        ?.querySelector<HTMLInputElement>("input:checked")
        ?.focus({ preventScroll: true });
    focus.restoreViewFocus.current = false;
  }, [focus, isFetching, selectedId, view]);

  const refresh = useRefreshReconcile({
    focus,
    list,
    selectedId,
    workId,
    onSelectionGone,
  });

  useLayoutEffect(() => {
    if (!focus.refreshFocusPending.current || selectedId !== null) return;
    focusFirst(focus, firstRow(focus));
    focus.refreshFocusPending.current = false;
  }, [focus, records, selectedId]);

  useLayoutEffect(() => {
    if (!focus.snoozeFocusPending.current || selectedId !== null || isFetching)
      return;
    focusFirst(focus, firstRow(focus));
    focus.snoozeFocusPending.current = false;
  }, [focus, isFetching, records, selectedId]);

  useLayoutEffect(() => {
    const pending = focus.dismissalFocusPending.current;
    if (
      !pending ||
      selectedId !== null ||
      records.some(({ value }) => value.id === pending.id)
    )
      return;
    const rows = visibleRows(focus.listRef.current);
    focusFirst(focus, rows.at(pending.index) ?? rows.at(-1));
    focus.dismissalFocusPending.current = null;
    focus.dismissedSelectionId.current = null;
  }, [focus, records, selectedId]);

  return refresh;
}
