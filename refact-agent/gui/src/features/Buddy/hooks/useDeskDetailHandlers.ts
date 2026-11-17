import type { Dispatch, SetStateAction } from "react";
import type { WorkItem } from "../operatorTypes";
import { headline } from "../buddyUtils";
import { visibleRows, type DeskFocusRefs } from "./useDeskFocusRestore";
import type { DeskSelection } from "./useDeskSelection";

export function useDeskDetailHandlers({
  focus,
  selection,
  selected,
  setRefreshNotice,
  setDismissalNotice,
  cancelRefresh,
  refetchList,
  refetchUnread,
}: {
  focus: DeskFocusRefs;
  selection: DeskSelection;
  selected: WorkItem | undefined;
  setRefreshNotice: Dispatch<SetStateAction<boolean>>;
  setDismissalNotice: Dispatch<SetStateAction<string | null>>;
  cancelRefresh: () => void;
  refetchList: () => void;
  refetchUnread: () => void;
}) {
  const rowIndex = (id: string) =>
    visibleRows(focus.listRef.current).findIndex(
      (row) => row.dataset.deskRow === id,
    );
  const back = () => {
    selection.clearSelection();
    refetchUnread();
    selection.setSelectedOverride(null);
    focus.snoozedSelectionId.current = null;
    selection.syncDeskLink(null);
  };
  const onDismissPending = (item: WorkItem) => {
    const index = rowIndex(item.id);
    focus.dismissalPendingId.current = item.id;
    focus.dismissalFocusPending.current = { id: item.id, index };
    selection.dismissSnapshotRef.current = item;
    selection.setSelectedOverride(item);
    cancelRefresh();
  };
  const onDismissFailure = (item: WorkItem) => {
    if (focus.dismissalPendingId.current !== item.id) return;
    focus.dismissalPendingId.current = null;
    selection.dismissSnapshotRef.current = null;
    focus.dismissalFocusPending.current = null;
    selection.setSelectedOverride(item);
  };
  const onDismiss = (item: WorkItem) => {
    focus.dismissalPendingId.current = null;
    selection.dismissSnapshotRef.current = null;
    focus.dismissalFocusPending.current ??= {
      id: item.id,
      index: rowIndex(item.id),
    };
    focus.dismissedSelectionId.current = item.id;
    focus.trigger.current = null;
    selection.setSelectedOverride(null);
    setRefreshNotice(false);
    setDismissalNotice(
      `Dismissed "${headline(item.title)}". You can find it under Delivered.`,
    );
    selection.clearSelection();
    selection.syncDeskLink(null);
  };
  const onSnoozeChange = () => {
    if (!selected) return;
    selection.setSelectedOverride(selected);
    focus.snoozedSelectionId.current = selected.id;
    focus.snoozeFocusPending.current = true;
    refetchList();
  };
  return {
    back,
    onDismiss,
    onDismissPending,
    onDismissFailure,
    onSnoozeChange,
  };
}
