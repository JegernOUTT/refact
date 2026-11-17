import { useRef, useState } from "react";
import {
  useGetBuddyOperatorWorkItemQuery,
  type OperatorRecord,
} from "../../../services/refact/buddyOperator";
import { useConfig } from "../../../hooks/useConfig";
import type { WorkItem } from "../operatorTypes";
import {
  BUDDY_REPORTS_SELECTION_KEY,
  type BuddyReportsSelection,
  type BuddyReportsView,
} from "../sections/buddyReportsLink";
import { useBuddyPersistentState } from "../ui";

export function useDeskSelection({
  endpointIdentity,
  workId,
  view,
}: {
  endpointIdentity: string;
  workId?: string;
  view: BuddyReportsView;
}) {
  const config = useConfig();
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
  const [selectedId, setSelectedId] = useState<string | null>(
    workId ?? restored?.workId ?? null,
  );
  const [selectedOverride, setSelectedOverride] = useState<WorkItem | null>(
    null,
  );
  const dismissSnapshotRef = useRef<WorkItem | null>(null);
  const restoredWork = useGetBuddyOperatorWorkItemQuery(
    {
      id: restored?.workId ?? "",
      project_id: restored?.projectId,
      endpoint_identity: endpointIdentity,
    },
    { skip: !restored || selectedId !== restored.workId },
  );

  const syncDeskLink = (item: WorkItem | null) => {
    if (config.host !== "web" || !config.engineServed) return;
    const url = new URL(window.location.href);
    if (item) {
      url.searchParams.set("buddy_work", item.id);
      url.searchParams.set("buddy_project", item.project.project_id);
      url.searchParams.delete("chat");
    } else {
      url.searchParams.delete("buddy_work");
      url.searchParams.delete("buddy_project");
    }
    window.history.replaceState(window.history.state, "", url);
  };

  const restoredValue = () => {
    const value = restoredWork.currentData?.data.value;
    return restored?.workId === selectedId &&
      value?.id === selectedId &&
      value.project.project_id === restored.projectId
      ? value
      : undefined;
  };
  const restoredActive = restored !== null && selectedId === restored.workId;
  const restoredSettled =
    restoredActive &&
    !restoredWork.isFetching &&
    (Boolean(restoredWork.error) || restoredWork.currentData !== undefined);

  const resolveSelected = (
    records: OperatorRecord<WorkItem>[],
    linkedValue: WorkItem | undefined,
  ): WorkItem | undefined => {
    if (selectedId === workId) return linkedValue;
    return (
      records.find((record) => record.value.id === selectedId)?.value ??
      (selectedOverride?.id === selectedId ? selectedOverride : undefined) ??
      (dismissSnapshotRef.current?.id === selectedId
        ? dismissSnapshotRef.current
        : undefined) ??
      restoredValue()
    );
  };

  const select = (item: WorkItem) => {
    setSelectedOverride(null);
    setSelectedId(item.id);
    setStoredSelection({
      workId: item.id,
      projectId: item.project.project_id,
      endpointIdentity,
      view,
    });
    syncDeskLink(item);
  };

  const clearSelection = () => {
    setSelectedId(null);
    setStoredSelection(null);
  };

  return {
    selectedId,
    setSelectedId,
    setSelectedOverride,
    dismissSnapshotRef,
    restoredWork,
    restoredLoading: restoredActive && restoredWork.isFetching,
    restoredFailed: restoredSettled && restoredValue() === undefined,
    resolveSelected,
    select,
    clearSelection,
    syncDeskLink,
  };
}

export type DeskSelection = ReturnType<typeof useDeskSelection>;
