import React, { useCallback } from "react";
import { FileText, Inbox, MessageCircle } from "lucide-react";
import {
  Badge,
  Button,
  Icon,
  LoadingState,
  Surface,
  Text,
} from "../../components/ui";
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
import { BuddySectionHeader } from "./BuddySectionHeader";
import { formatRelativeTime, headline, sameText } from "./buddyUtils";
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

export type BuddyHomeWorkMode = "enabled" | "legacy" | "loading" | "degraded";

const STATE_LABELS: Record<WorkItemState, string> = {
  needs_attention: "Needs you",
  blocked: "Blocked",
  working: "Working",
  delivered: "Delivered",
  watching: "Watching",
  cancelled: "Cancelled",
};

const STATE_TONES: Record<
  WorkItemState,
  React.ComponentProps<typeof Badge>["tone"]
> = {
  needs_attention: "accent",
  blocked: "danger",
  working: "muted",
  delivered: "success",
  watching: "muted",
  cancelled: "muted",
};

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

const WorkRow: React.FC<{
  item: WorkItem;
  onOpen: OpenWork;
}> = ({ item, onOpen }) => {
  const title = headline(item.title) || "Untitled report";
  const rawSummary = item.problem_summary?.trim();
  const summary =
    rawSummary &&
    !sameText(rawSummary, title) &&
    !sameText(headline(rawSummary), title)
      ? rawSummary
      : null;
  const freshness = formatRelativeTime(item.last_material_change_at);
  return (
    <li className={styles.row} data-state={item.state}>
      <span className={styles.rowTick} aria-hidden />
      <div className={styles.rowBody}>
        <div className={styles.rowTitle}>
          <Text as="h3" size="2" weight="bold" className={styles.rowHeading}>
            {title}
          </Text>
          <Badge size="xs" tone={STATE_TONES[item.state]}>
            {STATE_LABELS[item.state]}
          </Badge>
        </div>
        {summary && (
          <Text size="1" color="gray" className={styles.rowText}>
            {summary}
          </Text>
        )}
        <Text size="1" color="gray" className={styles.rowMeta}>
          {[item.scope?.trim(), freshness && `Changed ${freshness}`]
            .filter(Boolean)
            .join(" · ")}
        </Text>
      </div>
      <Button
        type="button"
        size="sm"
        variant="ghost"
        aria-label={`Open ${title}`}
        onClick={() => onOpen(item.id, item.project.project_id)}
      >
        Open
      </Button>
    </li>
  );
};

const QueryFailure: React.FC<{ text: string; onRetry: () => void }> = ({
  text,
  onRetry,
}) => (
  <div className={styles.failure} role="alert">
    <Text size="1" className={styles.rowText}>
      {text}
    </Text>
    <Button type="button" size="sm" variant="ghost" onClick={onRetry}>
      Retry
    </Button>
  </div>
);

const ModeNotice: React.FC<{ mode: BuddyHomeWorkMode; label: string }> = ({
  mode,
  label,
}) => {
  if (mode === "loading")
    return <LoadingState label={`Checking ${label}…`} variant="compact" />;
  return (
    <Text size="1" color="gray" className={styles.calm}>
      {mode === "degraded"
        ? `${label[0].toUpperCase()}${label.slice(
            1,
          )} cannot be listed while work authority is down. This is a connection failure, not an empty list.`
        : "Saved reports are off in this project. Pulse and opportunities are under Reports."}
    </Text>
  );
};

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
  const hasMore = query.currentData?.next_cursor != null;
  return (
    <Surface
      as="section"
      variant="glass"
      radius="card"
      className={styles.card}
      aria-label="Worth your attention"
      data-testid="buddy-home-saved-work"
    >
      <BuddySectionHeader icon={Inbox} label="Worth your attention" />
      {mode !== "enabled" ? (
        <ModeNotice mode={mode} label="saved work" />
      ) : Boolean(query.error) ||
        hasProjectMismatch(query.currentData, project) ? (
        <QueryFailure
          text="Saved work could not be loaded. This is a connection failure, not an empty queue."
          onRetry={() => void query.refetch()}
        />
      ) : !query.currentData ? (
        <LoadingState label="Loading saved work…" variant="compact" />
      ) : rows.length === 0 ? (
        <Text size="1" color="gray" className={styles.calm}>
          Nothing saved needs a decision right now.
        </Text>
      ) : (
        <ul className={styles.list} aria-label="Saved work preview">
          {rows.map((record) => (
            <WorkRow
              key={record.value.id}
              item={record.value}
              onOpen={onOpen}
            />
          ))}
        </ul>
      )}
      {mode === "enabled" && (
        <div className={styles.footer}>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={onShowReports}
          >
            {hasMore ? "More reports" : "All reports"}
          </Button>
        </div>
      )}
    </Surface>
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
  if (mode !== "enabled") return null;
  return (
    <Surface
      as="section"
      variant="glass"
      radius="card"
      className={styles.card}
      aria-label="Latest delivered report"
      data-testid="buddy-home-latest-report"
    >
      <BuddySectionHeader icon={FileText} label="Latest delivered report" />
      {Boolean(query.error) ||
      hasProjectMismatch(query.currentData, project) ? (
        <QueryFailure
          text="Delivered reports could not be loaded right now."
          onRetry={() => void query.refetch()}
        />
      ) : !query.currentData ? (
        <LoadingState label="Loading reports…" variant="compact" />
      ) : latest ? (
        <ul className={styles.list}>
          <WorkRow item={latest.value} onOpen={onOpen} />
        </ul>
      ) : (
        <Text size="1" color="gray" className={styles.calm}>
          No report to feature from the most recent saved runs. Housekeeping and
          older reports stay in Reports.
        </Text>
      )}
      <div className={styles.footer}>
        <Button type="button" size="sm" variant="ghost" onClick={onShowReports}>
          More reports
        </Button>
      </div>
    </Surface>
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
    <div className={styles.talk}>
      <Button
        type="button"
        size="sm"
        variant="primary"
        leftIcon={MessageCircle}
        loading={isLoading}
        onClick={() => void handleTalk()}
      >
        Talk to {name}
      </Button>
      {isError && (
        <Text size="1" color="red" role="alert">
          <Icon icon={MessageCircle} size="sm" tone="danger" /> The conversation
          did not start. Try again.
        </Text>
      )}
    </div>
  );
};
