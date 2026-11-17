import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import {
  useGetBuddyOperatorWorkQuery,
  type OperatorPage,
  type OperatorRecord,
} from "../../../services/refact/buddyOperator";
import type { WorkItem } from "../operatorTypes";
import {
  VIEW_FOR_STATE,
  groupDeskRecords,
  type DeskAttention,
} from "../BuddyDeskModel";
import type { BuddyReportsView } from "../sections/buddyReportsLink";

type WorkPage = OperatorPage<OperatorRecord<WorkItem>[]>;

export const DESK_SEARCH_DEBOUNCE_MS = 250;

function mergeRecords(
  previous: OperatorRecord<WorkItem>[],
  currentData: WorkPage | undefined,
): OperatorRecord<WorkItem>[] {
  const compatiblePrevious = currentData
    ? previous.filter(
        (record) =>
          record.value.project.project_id === currentData.project.project_id &&
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
}

function fromOtherProject(
  previous: OperatorRecord<WorkItem>[],
  currentData: WorkPage,
): boolean {
  return previous.some(
    (record) =>
      record.value.project.canonical_root !==
        currentData.project.canonical_root ||
      record.value.project.project_id !== currentData.project.project_id,
  );
}

export function useDeskWorkList({
  endpointIdentity,
  view,
  setSelectedId,
  onFiltersApplied,
}: {
  endpointIdentity: string;
  view: BuddyReportsView;
  setSelectedId: Dispatch<SetStateAction<string | null>>;
  onFiltersApplied: () => void;
}) {
  const [cursor, setCursor] = useState<number>();
  const [previous, setPrevious] = useState<OperatorRecord<WorkItem>[]>([]);
  const [search, setSearchInput] = useState("");
  const [appliedSearch, setAppliedSearch] = useState("");
  const [attention, setAttention] = useState<DeskAttention>("unsnoozed");
  const appliedRef = useRef(onFiltersApplied);
  appliedRef.current = onFiltersApplied;
  const applySearch = useCallback((value: string) => {
    setAppliedSearch(value);
    setCursor(undefined);
    setPrevious([]);
    appliedRef.current();
  }, []);
  const list = useGetBuddyOperatorWorkQuery(
    {
      cursor,
      limit: 50,
      endpoint_identity: endpointIdentity,
      view,
      search: appliedSearch,
      attention,
    },
    { refetchOnMountOrArgChange: true },
  );
  const { currentData, refetch } = list;
  const unreadSkipped = attention === "unread" || !currentData?.data.length;
  const unreadQuery = useGetBuddyOperatorWorkQuery(
    {
      limit: 200,
      endpoint_identity: endpointIdentity,
      view,
      search: appliedSearch,
      attention: "unread",
    },
    { skip: unreadSkipped },
  );
  const unreadIds = useMemo(
    () => new Set(unreadQuery.currentData?.data.map(({ value }) => value.id)),
    [unreadQuery.currentData],
  );
  const records = useMemo(
    () => mergeRecords(previous, currentData),
    [currentData, previous],
  );
  const query = appliedSearch.toLocaleLowerCase();
  const filtered = records.filter(
    ({ value }) => VIEW_FOR_STATE[value.state] === view,
  );
  const primary = filtered.filter(({ value }) => value.state !== "cancelled");
  const cancelled = filtered.filter(({ value }) => value.state === "cancelled");
  const groups = groupDeskRecords(primary);

  useEffect(() => {
    if (currentData && fromOtherProject(previous, currentData)) {
      setPrevious([]);
      setSelectedId(null);
      setCursor(undefined);
    }
  }, [currentData, previous, setSelectedId]);

  useEffect(() => {
    const next = search.trim();
    if (next === appliedSearch) return;
    const timer = window.setTimeout(
      () => applySearch(next),
      DESK_SEARCH_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(timer);
  }, [appliedSearch, applySearch, search]);

  return {
    list,
    search,
    attention,
    query,
    records,
    filtered,
    primary,
    cancelled,
    groups,
    filtersApplied: query !== "" || attention !== "unsnoozed",
    moreAvailable: currentData?.next_cursor != null,
    initialLoading: list.isFetching && !currentData && records.length === 0,
    unreadError: unreadSkipped ? undefined : unreadQuery.error,
    isUnread: (item: WorkItem) =>
      attention === "unread" || unreadIds.has(item.id),
    setSearch: (value: string) => {
      setSearchInput(value);
      if (value.trim() === "" && appliedSearch !== "") applySearch("");
    },
    setAttention,
    refetchUnread: () => {
      if (!unreadSkipped) void unreadQuery.refetch();
    },
    loadMore: () => {
      setPrevious(records);
      setCursor(currentData?.next_cursor ?? undefined);
    },
    resetPaging: () => {
      setCursor(undefined);
      setPrevious([]);
    },
    reload: () => {
      setPrevious([]);
      if (cursor === undefined) void refetch();
      else setCursor(undefined);
    },
  };
}

export type DeskWorkList = ReturnType<typeof useDeskWorkList>;
