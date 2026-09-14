import { useEffect, useRef, useState } from "react";
import { v4 as uuidv4 } from "uuid";
import { Zap } from "lucide-react";
import { Button, Surface, Text } from "../../components/ui";
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
import { BuddySectionHeader } from "./BuddySectionHeader";
import { BuddyResourceAction } from "./BuddyResourceAction";
import { buddyRegisteredActionHandler } from "./executeBuddyAction";
import styles from "./BuddyDeskActions.module.css";

function ActionControl({
  action,
  item,
  endpointIdentity,
  refresh,
  disabled,
}: {
  action: DeskAction;
  item: WorkItem;
  endpointIdentity: string;
  refresh: () => void;
  disabled: boolean;
}) {
  const [execute] = useExecuteBuddyDeskActionMutation();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<OperatorRecord<ActionExecution> | null>(
    null,
  );
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const key = useRef<string>();
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
  const supported =
    buddyRegisteredActionHandler(action.spec) === "saved_evidence";
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
      if (!mounted.current) return;
      setResult(response.data);
      key.current = undefined;
      refresh();
    } catch {
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
  return (
    <section className={styles.action} aria-label={action.label}>
      <Text as="p" size="2" className={styles.description}>
        {action.description}
      </Text>
      <Button
        size="sm"
        variant="primary"
        disabled={disabled || pending || !available}
        onClick={() => void run()}
      >
        {pending ? "Checking saved evidence…" : action.label}
      </Button>
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
            Saved check: {humanizeIdentifier(latest.status)}
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
          <details className={styles.disclosure}>
            <summary className={styles.summary}>Execution receipt</summary>
            <Text as="p" size="1" color="gray" className={styles.description}>
              {latest.execution_id}
            </Text>
            <Text as="p" size="1" color="gray" className={styles.description}>
              {latest.ended_at ?? "No terminal observation recorded yet."}
            </Text>
          </details>
        </div>
      )}
    </section>
  );
}

export function BuddyDeskActions({
  item,
  artifact,
  endpointIdentity,
}: {
  item: WorkItem;
  artifact: ArtifactRevision;
  endpointIdentity: string;
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
  if (!error && !isFetching && !stale && (actions?.length ?? 0) === 0) {
    return proposals ? (
      <Text as="p" size="1" color="gray" className={styles.footnote}>
        Buddy only describes what it found; no proposed fix is applied
        automatically.
      </Text>
    ) : null;
  }
  return (
    <Surface
      as="section"
      variant="surface-1"
      radius="card"
      className={styles.panel}
      aria-label="Registered actions"
    >
      <BuddySectionHeader icon={Zap} label="What you can do" />
      {isFetching && (
        <Text as="p" size="1" color="gray" role="status">
          Loading current actions…
        </Text>
      )}
      {error && (
        <div role="alert" className={styles.alert}>
          <Text as="p" size="2" color="red">
            Registered actions could not be loaded. Saved execution status is
            unavailable.
          </Text>
          <Button size="sm" onClick={() => void refetch()}>
            Retry actions
          </Button>
        </div>
      )}
      {!error &&
        actions?.map((action) =>
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
              refresh={() => void refetch()}
            />
          ),
        )}
      {stale && (
        <Text as="p" size="2" color="gray" className={styles.description}>
          No registered actions are available for this report revision.
        </Text>
      )}
      {!error && (
        <Button
          size="sm"
          variant="ghost"
          disabled={isFetching}
          onClick={() => void refetch()}
        >
          Reload action status
        </Button>
      )}
      {proposals && (
        <Text as="p" size="1" color="gray" className={styles.footnote}>
          Buddy only describes what it found; no proposed fix is applied
          automatically.
        </Text>
      )}
    </Surface>
  );
}
