import React, { useCallback } from "react";
import { FileText, Inbox, MessageCircle } from "lucide-react";
import { Button, Text } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push } from "../Pages/pagesSlice";
import { newBuddyChatAction, openBuddyChat } from "../Chat/Thread";
import { useCreateBuddyConversationMutation } from "../../services/refact/buddy";
import {
  buddyOperatorIdentity,
  useGetBuddyOperatorWorkQuery,
  type OperatorRecord,
} from "../../services/refact/buddyOperator";
import type { ProjectIdentity, WorkItem, WorkItemState } from "./operatorTypes";
import {
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyLoading,
  BuddyRow,
  BuddySection,
  BuddyStatusBadge,
  BuddyTimestamp,
  buddyStatus,
  buddyTalkTo,
  toBuddyWorkState,
} from "./ui";
import {
  requestBuddyReportsView,
  type BuddyReportsView,
} from "./sections/buddyReportsLink";
import { headline, sameText } from "./buddyUtils";
import styles from "./BuddyHomeOverview.module.css";

export const HOME_SAVED_LIMIT = 3;
const HOME_DELIVERED_SCAN = 10;
const HOUSEKEEPING_WORKFLOWS = new Set([
  "commit_message",
  "memo_extraction",
  "kg_enrich",
  "kg_deprecate",
  "title_generation",
]);
const SCOPE_ECHO_PREFIX = /^(latest|last|recent)\s+run\s+of\s+/i;

export type BuddyHomeWorkMode = "enabled" | "legacy" | "loading" | "degraded";

type OpenWork = (workId: string, projectId: string) => void;

type WorkPage = {
  project: ProjectIdentity;
  data: OperatorRecord<WorkItem>[];
};

const sameProject = (left: ProjectIdentity, right: ProjectIdentity) =>
  left.project_id === right.project_id &&
  left.canonical_root === right.canonical_root;

function hasProjectMismatch(
  page: WorkPage | undefined,
  expected: ProjectIdentity | null,
): boolean {
  return Boolean(
    page &&
      ((expected !== null && !sameProject(page.project, expected)) ||
        page.data.some(
          (record) => !sameProject(record.value.project, page.project),
        )),
  );
}

function projectRecords(
  page: WorkPage | undefined,
  expected: ProjectIdentity | null,
): OperatorRecord<WorkItem>[] {
  if (!page) return [];
  if (expected && !sameProject(page.project, expected)) return [];
  return page.data.filter((record) =>
    sameProject(record.value.project, page.project),
  );
}

function normalized(value: string): string {
  return value.trim().replace(/\s+/g, " ").toLowerCase();
}

function scopeEchoesTitle(scope: string, title: string): boolean {
  const left = normalized(scope.replace(SCOPE_ECHO_PREFIX, ""));
  const right = normalized(title);
  if (!left) return true;
  return left === right || right.includes(left) || left.includes(right);
}

const WorkRow: React.FC<{
  item: WorkItem;
  listState: WorkItemState;
  onOpen: OpenWork;
}> = ({ item, listState, onOpen }) => {
  const title = headline(item.title) || "Untitled report";
  const rawSummary = item.problem_summary?.trim();
  const summary =
    rawSummary &&
    !sameText(rawSummary, title) &&
    !sameText(headline(rawSummary), title)
      ? rawSummary
      : null;
  const scope = item.scope?.trim();
  const workState = toBuddyWorkState(item.state) ?? "watching";
  return (
    <li className={styles.item} data-state={item.state}>
      <BuddyRow
        title={title}
        titleLines={2}
        leading={{ tick: buddyStatus("work", workState).tone }}
        status={
          item.state !== listState ? (
            <BuddyStatusBadge dimension="work" value={workState} />
          ) : undefined
        }
        description={summary}
        meta={[
          scope && !scopeEchoesTitle(scope, title) ? scope : null,
          <BuddyTimestamp
            key="changed"
            value={item.last_material_change_at}
            prefix="Changed"
          />,
        ]}
        aria-label={`Open ${title}`}
        onClick={() => onOpen(item.id, item.project.project_id)}
      />
    </li>
  );
};

const ModeNotice: React.FC<{ mode: BuddyHomeWorkMode }> = ({ mode }) => {
  if (mode === "loading")
    return <BuddyLoading variant="rows" count={2} label="Loading reports" />;
  return (
    <BuddyEmptyState
      compact
      icon={Inbox}
      title={
        mode === "degraded"
          ? "Reports can't be listed while the connection is down."
          : "Reports are off in this project."
      }
      hint={
        mode === "degraded"
          ? "This is a connection failure, not an empty list."
          : "Pulse and opportunities are under Reports."
      }
    />
  );
};

function showReportsView(
  view: BuddyReportsView,
  onShowReports: () => void,
): () => void {
  return () => {
    requestBuddyReportsView(view);
    onShowReports();
  };
}

export const BuddyHomeSavedWork: React.FC<{
  mode: BuddyHomeWorkMode;
  project: ProjectIdentity | null;
  onOpen: OpenWork;
  onShowReports: () => void;
}> = ({ mode, project, onOpen, onShowReports }) => {
  const identity = useAppSelector(buddyOperatorIdentity);
  const query = useGetBuddyOperatorWorkQuery(
    {
      limit: HOME_SAVED_LIMIT,
      endpoint_identity: identity,
      view: "needs_you",
      attention: "unsnoozed",
    },
    { skip: mode !== "enabled", refetchOnMountOrArgChange: true },
  );
  const rows = projectRecords(query.currentData, project).slice(
    0,
    HOME_SAVED_LIMIT,
  );
  const mismatch = hasProjectMismatch(query.currentData, project);
  return (
    <BuddySection
      title="Worth your attention"
      icon={Inbox}
      testId="buddy-home-saved-work"
      action={
        mode === "enabled" ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={showReportsView("needs_you", onShowReports)}
          >
            View all reports
          </Button>
        ) : undefined
      }
    >
      {mode !== "enabled" ? (
        <ModeNotice mode={mode} />
      ) : Boolean(query.error) || mismatch ? (
        <BuddyErrorCallout
          compact
          thing="reports"
          error={query.error}
          details={
            mismatch ? "The reports belong to another project." : undefined
          }
          retrying={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !query.currentData ? (
        <BuddyLoading variant="rows" count={2} label="Loading reports" />
      ) : rows.length === 0 ? (
        <BuddyEmptyState
          compact
          icon={Inbox}
          title="Nothing needs you right now."
        />
      ) : (
        <ul className={styles.list} aria-label="Reports that need you">
          {rows.map((record) => (
            <WorkRow
              key={record.value.id}
              item={record.value}
              listState="needs_attention"
              onOpen={onOpen}
            />
          ))}
        </ul>
      )}
    </BuddySection>
  );
};

export const BuddyHomeLatestReport: React.FC<{
  mode: BuddyHomeWorkMode;
  project: ProjectIdentity | null;
  onOpen: OpenWork;
  onShowReports: () => void;
}> = ({ mode, project, onOpen, onShowReports }) => {
  const identity = useAppSelector(buddyOperatorIdentity);
  const query = useGetBuddyOperatorWorkQuery(
    {
      limit: HOME_DELIVERED_SCAN,
      endpoint_identity: identity,
      view: "delivered",
      attention: "all",
    },
    { skip: mode !== "enabled", refetchOnMountOrArgChange: true },
  );
  const latest = projectRecords(query.currentData, project).find(
    ({ value }) =>
      value.state === "delivered" &&
      !(
        value.subject.resource_kind === "workflow" &&
        HOUSEKEEPING_WORKFLOWS.has(value.subject.resource_id)
      ),
  );
  const mismatch = hasProjectMismatch(query.currentData, project);
  if (mode !== "enabled") return null;
  return (
    <BuddySection
      title="Latest delivered report"
      icon={FileText}
      testId="buddy-home-latest-report"
      action={
        <Button
          type="button"
          size="sm"
          variant="ghost"
          onClick={showReportsView("delivered", onShowReports)}
        >
          View delivered
        </Button>
      }
    >
      {Boolean(query.error) || mismatch ? (
        <BuddyErrorCallout
          compact
          thing="delivered reports"
          error={query.error}
          details={
            mismatch ? "The reports belong to another project." : undefined
          }
          retrying={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !query.currentData ? (
        <BuddyLoading variant="rows" count={1} label="Loading reports" />
      ) : latest ? (
        <ul className={styles.list} aria-label="Latest delivered report">
          <WorkRow item={latest.value} listState="delivered" onOpen={onOpen} />
        </ul>
      ) : (
        <BuddyEmptyState
          compact
          icon={FileText}
          title="No delivered reports yet."
          hint="Housekeeping runs and older reports stay in Reports."
        />
      )}
    </BuddySection>
  );
};

export const BuddyHomeTalk: React.FC<{ name: string }> = ({ name }) => {
  const dispatch = useAppDispatch();
  const [createConversation, { isLoading, isError }] =
    useCreateBuddyConversationMutation();
  const handleTalk = useCallback(async () => {
    const result = await createConversation(undefined);
    if ("data" in result && result.data) {
      dispatch(newBuddyChatAction({ chat_id: result.data.chat_id }));
      dispatch(
        openBuddyChat({
          chat_id: result.data.chat_id,
          title: result.data.title,
        }),
      );
      dispatch(push({ name: "chat" }));
    }
  }, [createConversation, dispatch]);
  return (
    <>
      <Button
        type="button"
        size="sm"
        variant="primary"
        leftIcon={MessageCircle}
        loading={isLoading}
        className={styles.talk}
        onClick={() => void handleTalk()}
      >
        {buddyTalkTo(name)}
      </Button>
      {isError && (
        <Text size="1" role="alert" className={styles.talkError}>
          The conversation didn&apos;t start. Try again.
        </Text>
      )}
    </>
  );
};
