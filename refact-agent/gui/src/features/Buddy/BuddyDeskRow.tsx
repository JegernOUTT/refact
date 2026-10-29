import { useEffect } from "react";
import { useGetBuddyDeskActionsQuery } from "../../services/refact/buddyOperator";
import type { WorkItem } from "./operatorTypes";
import { headline } from "./buddyUtils";
import {
  ACTION_STATUS_UNAVAILABLE,
  NO_ACTION,
  NO_RESULT_YET,
  type RowActions,
} from "./BuddyDeskModel";
import {
  BuddyRow,
  BuddyStatusBadge,
  BuddyTimestamp,
  buddyStatus,
  toBuddySeverity,
  toBuddyWorkState,
} from "./ui";
import styles from "./BuddyDesk.module.css";

const opaqueIdentifier = /[0-9a-f]{24,}/i;
const SCOPE_ECHO_PREFIX = /^latest run of\s+/i;

function meaningfulText(value: string | null | undefined): string | null {
  const text = value?.trim() ?? "";
  if (text === "" || opaqueIdentifier.test(text)) return null;
  return text;
}

function comparableText(value: string): string {
  return value
    .toLocaleLowerCase()
    .replace(SCOPE_ECHO_PREFIX, "")
    .replace(/[^a-z0-9]+/g, " ")
    .replace(
      /\b(?:a|an|the|issue|problem|finding|report|review|assessment)\b/g,
      " ",
    )
    .replace(/\s+/g, " ")
    .trim();
}

function echoes(value: string, title: string): boolean {
  const left = comparableText(value);
  const right = comparableText(title);
  return (
    left === "" ||
    left === right ||
    left.includes(right) ||
    right.includes(left)
  );
}

function problemSummary(item: WorkItem): string | null {
  const summary = meaningfulText(item.problem_summary);
  if (!summary || echoes(summary, item.title)) return null;
  return summary;
}

function rowScope(item: WorkItem): string | null {
  const scope = meaningfulText(item.scope);
  if (!scope || echoes(scope, headline(item.title))) return null;
  return scope;
}

export function DeskRow({
  item,
  endpointIdentity,
  defaultState,
  showActionVerb,
  rowActions,
  selected,
  unread,
  onOpen,
  onActionVerb,
}: {
  item: WorkItem;
  endpointIdentity: string;
  defaultState: WorkItem["state"];
  showActionVerb: boolean;
  rowActions: RowActions;
  selected: boolean;
  unread: boolean;
  onOpen: (item: WorkItem, trigger: HTMLElement) => void;
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
  useEffect(() => {
    if (verb !== null) onActionVerb(item.id, verb);
  }, [item.id, onActionVerb, verb]);

  const title = headline(item.title) || "Untitled report";
  const workState = toBuddyWorkState(item.state) ?? "watching";
  const severity = toBuddySeverity(item.severity);
  const status = (
    <>
      {item.state !== defaultState && (
        <BuddyStatusBadge dimension="work" value={workState} />
      )}
      {severity && (
        <BuddyStatusBadge
          dimension="severity"
          value={severity}
          testId="buddy-desk-row-severity"
        />
      )}
    </>
  );
  return (
    <li className={styles.rowItem}>
      <BuddyRow
        data-desk-row={item.id}
        title={<span data-testid="buddy-desk-row-title">{title}</span>}
        fullTitle={title}
        titleLines={2}
        leading={{ tick: buddyStatus("work", workState).tone }}
        status={item.state !== defaultState || severity ? status : undefined}
        description={problemSummary(item)}
        meta={[
          rowScope(item),
          showActionVerb && verb !== null ? verb : null,
          <BuddyTimestamp
            key="changed"
            value={item.last_material_change_at}
            prefix="Changed"
          />,
        ]}
        selected={selected}
        unread={unread}
        onClick={(event) => onOpen(item, event.currentTarget)}
      />
    </li>
  );
}
