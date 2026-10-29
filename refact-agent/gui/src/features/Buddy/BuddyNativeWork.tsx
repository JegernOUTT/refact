import { useRef, useState } from "react";
import { Activity, RefreshCw, Users } from "lucide-react";
import { Button, IconButton, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import {
  useGetBuddyNativeWorkQuery,
  type NativeWorkView,
} from "../../services/refact/buddyNativeWork";
import { humanizeIdentifier } from "../../utils/displayNames";
import {
  BUDDY_TERMS,
  BuddyDisclosure,
  BuddyEmptyState,
  BuddyErrorCallout,
  BuddyLoading,
  BuddyRow,
  BuddySection,
  useBuddyPersistentState,
} from "./ui";
import styles from "./BuddyNativeDecisions.module.css";

const BUDDY_BACKGROUND_WORK_KEY = "buddy.home.backgroundWork";

function modelLabel(model: string): string {
  return model.split("/").filter(Boolean).at(-1) ?? model;
}

function limitText(used: number, max: number | null): string {
  return max === null ? `${used}` : `${used} of ${max}`;
}

function sessionNotes(row: NativeWorkView): string[] {
  return [
    row.pending_questions > 0 &&
      `${row.pending_questions} question set${
        row.pending_questions === 1 ? "" : "s"
      } need a response.`,
    row.approval_count > 0 &&
      `${row.approval_count} tool or privacy approvals are waiting in the chat.`,
    row.browser_decision_pending &&
      "A browser-context choice is waiting in the chat.",
    row.persistence_pending &&
      "Live changes are awaiting persistence and may not survive a restart.",
  ].filter((note): note is string => Boolean(note));
}

function SessionGoal({ goal }: { goal: NonNullable<NativeWorkView["goal"]> }) {
  return (
    <div className={styles.workGoal}>
      <Text
        as="p"
        size="2"
        className={styles.workGoalText}
        title={goal.content}
      >
        {goal.content}
      </Text>
      <Text as="p" size="1" className={styles.note}>
        {[
          `Goal ${humanizeIdentifier(goal.status).toLowerCase()}`,
          goal.active ? "owned by this chat" : "not owned by this chat",
          `turns ${limitText(goal.turns_used, goal.max_turns)}`,
          `tokens ${limitText(goal.tokens_used, goal.max_tokens)}`,
        ].join(" · ")}
      </Text>
      <Text as="p" size="1" className={styles.note}>
        {goal.latest_verdict
          ? `Latest check: ${goal.latest_verdict}`
          : "Not checked yet. Worker completion is not verification."}
      </Text>
      {goal.verification_blocked && (
        <Text as="p" size="1" className={styles.note}>
          Verification is blocked, so completion isn&apos;t claimed.
        </Text>
      )}
      {goal.gaps.map((gap, index) => (
        <Text as="p" size="1" className={styles.note} key={index}>
          {gap}
        </Text>
      ))}
      {goal.truncated && (
        <Text as="p" size="1" className={styles.note}>
          Shortened here. The full goal is in the chat.
        </Text>
      )}
    </div>
  );
}

function SessionCard({ row }: { row: NativeWorkView }) {
  return (
    <li className={styles.workItem}>
      <BuddyRow
        title={row.title.trim() || "Untitled chat"}
        meta={[
          humanizeIdentifier(row.state),
          row.model ? modelLabel(row.model) : "No model selected",
        ]}
      />
      <div className={styles.workBody}>
        {sessionNotes(row).map((note) => (
          <Text as="p" size="1" className={styles.note} key={note}>
            {note}
          </Text>
        ))}
        {row.error && (
          <Text as="p" size="1" role="alert" className={styles.alert}>
            {row.error}
          </Text>
        )}
        <BuddyDisclosure
          label="Goal and workers"
          summary={
            row.goal
              ? `Goal ${humanizeIdentifier(row.goal.status).toLowerCase()}`
              : undefined
          }
          lazy
        >
          <div className={styles.workDetails}>
            {row.goal && <SessionGoal goal={row.goal} />}
            {row.agents.length > 0 && (
              <BuddyDisclosure
                label="Workers"
                icon={Users}
                count={row.agents.length}
              >
                <ul className={styles.workList}>
                  {row.agents.map((agent) => (
                    <li key={agent.agent_id}>
                      <BuddyRow
                        density="compact"
                        title={agent.title || "Worker"}
                        description={agent.progress}
                        meta={[
                          humanizeIdentifier(agent.status),
                          agent.model
                            ? modelLabel(agent.model)
                            : "Model not reported",
                        ]}
                      />
                    </li>
                  ))}
                </ul>
                {row.agents_truncated && (
                  <Text as="p" size="1" className={styles.note}>
                    Only the first twenty workers are shown.
                  </Text>
                )}
              </BuddyDisclosure>
            )}
            <BuddyDisclosure label={BUDDY_TERMS.technicalDetails}>
              <Text as="p" size="1" className={styles.note}>
                Chat ID: {row.chat_id}
                {row.model ? ` · Model: ${row.model}` : ""}
              </Text>
            </BuddyDisclosure>
          </div>
        </BuddyDisclosure>
      </div>
    </li>
  );
}

function NativeWork({ identity }: { identity: string }) {
  const [open, setOpen] = useBuddyPersistentState(
    BUDDY_BACKGROUND_WORK_KEY,
    false,
  );
  const [after, setAfter] = useState<string>();
  const refetchRef = useRef<() => void>(() => undefined);
  const visible = useIsDocumentVisible(() => refetchRef.current());
  const query = useGetBuddyNativeWorkQuery(
    { endpoint_identity: identity, after },
    {
      skip: !open,
      pollingInterval: visible ? 5000 : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
    },
  );
  refetchRef.current = () => {
    if (open) void query.refetch();
  };
  const data = query.currentData;
  const sessions = data?.sessions;
  const nextCursor = data?.next_cursor ?? null;
  return (
    <BuddySection
      title={BUDDY_TERMS.backgroundWork}
      icon={Activity}
      count={sessions?.length}
      testId="buddy-home-background-work"
      collapsible={{ open, onOpenChange: setOpen, lazy: true }}
      action={
        open ? (
          <IconButton
            size="sm"
            variant="ghost"
            icon={RefreshCw}
            aria-label="Refresh background work"
            disabled={query.isFetching}
            onClick={() => void query.refetch()}
          />
        ) : undefined
      }
      description="Live chats, goals and workers in the chats open right now. Read-only."
      footer={
        nextCursor !== null || after !== undefined ? (
          <>
            {after !== undefined && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setAfter(undefined)}
              >
                First page
              </Button>
            )}
            {nextCursor !== null && (
              <Button
                size="sm"
                variant="ghost"
                disabled={query.isFetching}
                onClick={() => setAfter(nextCursor)}
              >
                Show more
              </Button>
            )}
          </>
        ) : undefined
      }
    >
      {query.error ? (
        <BuddyErrorCallout
          compact
          thing="background work"
          error={query.error}
          retrying={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !sessions ? (
        <BuddyLoading
          variant="rows"
          count={2}
          label="Loading background work"
        />
      ) : sessions.length === 0 ? (
        <BuddyEmptyState
          compact
          icon={Activity}
          title="No background work in open chats."
        />
      ) : (
        <ul className={styles.workList}>
          {sessions.map((row) => (
            <SessionCard key={row.chat_id} row={row} />
          ))}
        </ul>
      )}
    </BuddySection>
  );
}

export function BuddyNativeWork() {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <NativeWork key={identity} identity={identity} />;
}
