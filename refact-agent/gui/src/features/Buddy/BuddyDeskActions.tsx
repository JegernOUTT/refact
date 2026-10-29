import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { v4 as uuidv4 } from "uuid";
import { Button, Text, Tooltip } from "../../components/ui";
import {
  useGetBuddyDeskActionsQuery,
  useExecuteBuddyDeskActionMutation,
  type DeskAction,
  type OperatorRecord,
} from "../../services/refact/buddyOperator";
import type {
  ActionExecution,
  ArtifactRevision,
  WorkItem,
} from "./operatorTypes";
import { humanizeIdentifier } from "../../utils/displayNames";
import { BuddyResourceAction } from "./BuddyResourceAction";
import { buddyRegisteredActionHandler } from "./executeBuddyAction";
import {
  BUDDY_TERMS,
  BuddyDisclosure,
  BuddyErrorCallout,
  BuddyTimestamp,
} from "./ui";
import styles from "./BuddyDeskActions.module.css";

function ActionControl({
  action,
  item,
  endpointIdentity,
  refresh,
  disabled,
  onDismiss,
  onDismissPending,
  onDismissFailure,
  inline = false,
}: {
  inline?: boolean;
  action: DeskAction;
  item: WorkItem;
  endpointIdentity: string;
  refresh: () => void;
  disabled: boolean;
  onDismiss?: (item: WorkItem) => void;
  onDismissPending?: (item: WorkItem) => void;
  onDismissFailure?: (item: WorkItem) => void;
}) {
  const [execute] = useExecuteBuddyDeskActionMutation();
  const descriptionId = useId();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<OperatorRecord<ActionExecution> | null>(
    null,
  );
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const key = useRef<string>();
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;
  const itemRef = useRef(item);
  itemRef.current = item;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const saved = action.latest_execution;
  const latest =
    result && (!saved || result.change_seq > saved.change_seq)
      ? result.value
      : saved?.value;
  const unresolved =
    latest &&
    ["claimed", "running", "reconciling", "outcome_unknown"].includes(
      latest.status,
    );
  const handler = buddyRegisteredActionHandler(action.spec);
  const lifecycle = handler === "work_lifecycle";
  const supported = handler === "saved_evidence" || lifecycle;
  const available =
    supported &&
    !unresolved &&
    action.spec.availability === "available" &&
    action.spec.capabilities.can_execute;
  const run = async () => {
    if (inFlight.current || disabled || !available) return;
    inFlight.current = true;
    setPending(true);
    setError(null);
    key.current ??= uuidv4();
    if (lifecycle && action.spec.action_id === "dismiss_not_a_problem")
      onDismissPending?.(item);
    try {
      const response = await execute({
        id: item.id,
        endpoint_identity: endpointIdentity,
        project_id: item.project.project_id,
        canonical_root: item.project.canonical_root,
        action_id: action.spec.action_id,
        work_revision: action.work_revision,
        artifact_id: action.artifact_id,
        artifact_revision: action.artifact_revision,
        spec_hash: action.spec_hash,
        idempotency_key: key.current,
      }).unwrap();
      const dismissed =
        lifecycle &&
        action.spec.action_id === "dismiss_not_a_problem" &&
        response.data.value.status === "completed";
      if (dismissed && onDismissRef.current) {
        onDismissRef.current(itemRef.current);
        key.current = undefined;
        return;
      }
      if (lifecycle && action.spec.action_id === "dismiss_not_a_problem") {
        onDismissFailure?.(itemRef.current);
        if (!mounted.current) return;
        setResult(response.data);
        key.current = undefined;
        refresh();
        return;
      }
      if (!mounted.current) return;
      setResult(response.data);
      key.current = undefined;
      refresh();
    } catch {
      if (lifecycle && action.spec.action_id === "dismiss_not_a_problem")
        onDismissFailure?.(itemRef.current);
      if (mounted.current) {
        setError(
          "No acknowledged result was received. Reload the saved status before retrying; a retry keeps the same request identity.",
        );
      }
    } finally {
      inFlight.current = false;
      if (mounted.current) setPending(false);
    }
  };
  const button = (
    <Button
      size="sm"
      variant={inline ? "ghost" : lifecycle ? "soft" : "primary"}
      disabled={disabled || pending || !available}
      aria-describedby={inline ? descriptionId : undefined}
      onClick={() => void run()}
    >
      {pending
        ? lifecycle
          ? "Dismissing item…"
          : "Verifying saved evidence…"
        : action.label}
    </Button>
  );
  return (
    <section
      className={inline ? styles.actionInline : styles.action}
      aria-label={action.label}
    >
      {inline ? (
        <>
          <span id={descriptionId} className={styles.srOnly}>
            {action.description}
          </span>
          <Tooltip content={action.description}>{button}</Tooltip>
        </>
      ) : (
        <>
          <Text as="p" size="2" className={styles.description}>
            {action.description}
          </Text>
          {button}
        </>
      )}
      {!supported && (
        <Text as="p" size="1" color="gray" className={styles.description}>
          This action type is not supported by this client.
        </Text>
      )}
      {error && (
        <Text as="p" size="1" color="red" role="alert">
          {error}
        </Text>
      )}
      {latest && (
        <div role="status" className={styles.receipt}>
          <Text as="p" size="1" weight="medium" className={styles.description}>
            {lifecycle ? "Dismissed from Needs you" : "Saved evidence check"}:{" "}
            {humanizeIdentifier(latest.status)}
          </Text>
          <ul className={styles.evidence}>
            {latest.verification_evidence.map((evidence, index) => (
              <li key={index}>
                <Text size="1" color="gray">
                  {evidence}
                </Text>
              </li>
            ))}
          </ul>
          <BuddyDisclosure label={BUDDY_TERMS.technicalDetails} lazy>
            <Text as="p" size="1" color="gray" className={styles.description}>
              Receipt {latest.execution_id}
            </Text>
            <Text as="p" size="1" color="gray" className={styles.description}>
              {latest.ended_at ? (
                <BuddyTimestamp
                  value={latest.ended_at}
                  mode="absolute"
                  prefix="Finished"
                />
              ) : (
                "Not finished yet."
              )}
            </Text>
          </BuddyDisclosure>
        </div>
      )}
    </section>
  );
}

export function BuddyDeskActions({
  item,
  artifact,
  endpointIdentity,
  onDismiss,
  onDismissPending,
  onDismissFailure,
  secondary,
}: {
  item: WorkItem;
  artifact: ArtifactRevision;
  endpointIdentity: string;
  onDismiss?: (item: WorkItem) => void;
  onDismissPending?: (item: WorkItem) => void;
  onDismissFailure?: (item: WorkItem) => void;
  secondary?: ReactNode;
}) {
  const { currentData, error, isFetching, refetch } =
    useGetBuddyDeskActionsQuery(
      {
        id: item.id,
        project_id: item.project.project_id,
        endpoint_identity: endpointIdentity,
      },
      {
        refetchOnMountOrArgChange: true,
        refetchOnFocus: true,
        refetchOnReconnect: true,
      },
    );
  const actions = currentData?.data.filter(
    (action) =>
      action.artifact_id === artifact.artifact_id &&
      action.artifact_revision === artifact.revision &&
      action.spec.project.canonical_root === item.project.canonical_root,
  );
  const returned = currentData?.data.length ?? 0;
  const stale = !isFetching && !error && returned > 0 && actions?.length === 0;
  const proposals = artifact.proposed_action_ids.length > 0;
  const footnote = proposals ? (
    <Text as="p" size="1" color="gray" className={styles.footnote}>
      Suggested fixes are never applied automatically.
    </Text>
  ) : null;
  const empty = !error && !isFetching && !stale && (actions?.length ?? 0) === 0;
  if (empty && !secondary) return footnote;
  const control = (action: DeskAction, inline: boolean) =>
    buddyRegisteredActionHandler(action.spec) === "resource" ? (
      <BuddyResourceAction
        key={`${endpointIdentity}:${item.id}:${action.spec_hash}`}
        spec={action.spec}
        label={action.label}
        endpointIdentity={endpointIdentity}
        disabled={isFetching}
        latestExecution={action.latest_execution?.value ?? null}
      />
    ) : (
      <ActionControl
        key={`${endpointIdentity}:${item.id}:${action.spec_hash}`}
        action={action}
        item={item}
        endpointIdentity={endpointIdentity}
        disabled={isFetching}
        inline={inline}
        refresh={() => void refetch()}
        onDismiss={onDismiss}
        onDismissPending={onDismissPending}
        onDismissFailure={onDismissFailure}
      />
    );
  const isLifecycle = (action: DeskAction) =>
    buddyRegisteredActionHandler(action.spec) === "work_lifecycle";
  const shown = error ? [] : actions ?? [];
  const primary = shown.filter((action) => !isLifecycle(action));
  const lifecycle = shown.filter(isLifecycle);
  return (
    <section className={styles.panel} aria-label="Report actions">
      {isFetching && (
        <Text as="p" size="1" color="gray" role="status">
          Loading actions…
        </Text>
      )}
      {error && (
        <BuddyErrorCallout
          thing="actions for this report"
          error={error}
          onRetry={() => void refetch()}
          retryLabel="Retry actions"
          compact
        />
      )}
      {primary.map((action) => control(action, false))}
      {stale && (
        <Text as="p" size="2" color="gray" className={styles.description}>
          No actions are available for this report revision.
        </Text>
      )}
      <div className={styles.secondary}>
        {lifecycle.map((action) => control(action, true))}
        {secondary}
        {!error && !empty && (
          <Button
            size="sm"
            variant="ghost"
            disabled={isFetching}
            onClick={() => void refetch()}
          >
            Refresh actions
          </Button>
        )}
      </div>
      {footnote}
    </section>
  );
}
