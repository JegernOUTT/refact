import { useCallback, useLayoutEffect, useState } from "react";
import type { OperatorRecord } from "../../../services/refact/buddyOperator";
import type { WorkItem } from "../operatorTypes";
import {
  CANCELLED_GROUP,
  type DeskListMode,
  type DeskRecordGroup,
} from "../BuddyDeskModel";

function groupOf(
  selectedId: string | null,
  listMode: DeskListMode,
  groups: DeskRecordGroup[],
  cancelled: OperatorRecord<WorkItem>[],
): string | null {
  if (selectedId === null) return null;
  if (cancelled.some(({ value }) => value.id === selectedId))
    return CANCELLED_GROUP;
  if (listMode !== "latest") return null;
  return (
    groups.find((group) =>
      group.others.some(({ value }) => value.id === selectedId),
    )?.key ?? null
  );
}

export function useDeskGroups({
  selectedId,
  groups,
  cancelled,
}: {
  selectedId: string | null;
  groups: DeskRecordGroup[];
  cancelled: OperatorRecord<WorkItem>[];
}) {
  const [listMode, setListMode] = useState<DeskListMode>("latest");
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
  const selectedGroup = groupOf(selectedId, listMode, groups, cancelled);
  useLayoutEffect(() => {
    if (selectedGroup !== null) toggleGroup(selectedGroup, true);
  }, [selectedGroup, toggleGroup]);
  return { listMode, setListMode, openGroups, toggleGroup };
}
