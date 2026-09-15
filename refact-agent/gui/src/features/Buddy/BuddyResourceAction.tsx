import { useRef, useState } from "react";
import { v4 as uuidv4 } from "uuid";
import { Button } from "../../components/ui";
import { humanizeIdentifier } from "../../utils/displayNames";
import type { ActionExecution, ActionSpec } from "./operatorTypes";
import {
  usePreviewBuddyResourceMutation,
  usePrepareBuddyResourceMutation,
  useRightsBuddyResourceMutation,
  useGrantBuddyResourceMutation,
  useExecuteBuddyResourceMutation,
  useLazyStatusBuddyResourceQuery,
  type ResourcePreview,
  type NativeRights,
  type PreparedResource,
} from "../../services/refact/buddyResourceActions";
import styles from "./BuddyResourceAction.module.css";

type Tracking = {
  execution_id: string;
  spec_hash: string;
  recovery_execution_id: string | null;
  recovery_id: string | null;
  idempotency_key: string;
};
function readTracking(key: string): Tracking | null {
  const raw = localStorage.getItem(key);
  if (raw === null) return null;
  const value: unknown = JSON.parse(raw);
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Invalid saved tracking");
  const row = value as Record<string, unknown>;
  if (
    Object.keys(row).length !== 5 ||
    ["execution_id", "spec_hash", "idempotency_key"].some(
      (key) => typeof row[key] !== "string" || !row[key].trim(),
    ) ||
    ["recovery_id", "recovery_execution_id"].some(
      (key) =>
        row[key] !== null && (typeof row[key] !== "string" || !row[key].trim()),
    )
  )
    throw new Error("Saved tracking is unreadable; execution remains blocked.");
  return row as Tracking;
}

/** Only render specs supplied by the registered server action list, never report prose. */
export function BuddyResourceAction({
  spec,
  endpointIdentity,
  label,
  disabled = false,
  latestExecution = null,
}: {
  spec: ActionSpec;
  endpointIdentity: string;
  label: string;
  disabled?: boolean;
  latestExecution?: ActionExecution | null;
}) {
  return (
    <ResourceControl
      key={JSON.stringify([endpointIdentity, spec])}
      spec={spec}
      endpointIdentity={endpointIdentity}
      label={label}
      disabled={disabled}
      latestExecution={latestExecution}
    />
  );
}
function ResourceControl({
  spec,
  endpointIdentity,
  label,
  disabled,
  latestExecution,
}: {
  spec: ActionSpec;
  endpointIdentity: string;
  label: string;
  disabled: boolean;
  latestExecution: ActionExecution | null;
}) {
  const context = { spec, endpoint_identity: endpointIdentity };
  const storageKey = `buddy.resource.execution:${JSON.stringify([
    endpointIdentity,
    spec.project,
    spec.action_id,
    spec.parameter_ref,
    spec.expected_version,
  ])}`;
  const [initial] = useState(() => {
    try {
      return { tracking: readTracking(storageKey), blocked: false };
    } catch {
      return { tracking: null, blocked: true };
    }
  });
  const [tracking, setTracking] = useState<Tracking | null>(initial.tracking);
  const [execution, setExecution] = useState<ActionExecution | null>(
    latestExecution,
  );
  const [preview, setPreview] = useState<ResourcePreview | null>(null);
  const [prepared, setPrepared] = useState<PreparedResource | null>(null);
  const [rights, setRights] = useState<NativeRights>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [rightsConfirmed, setRightsConfirmed] = useState(false);
  const [recoverConfirmed, setRecoverConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const lock = useRef(false);
  const [error, setError] = useState<string | null>(
    initial.blocked
      ? "Persistent execution tracking is unavailable. Actions are blocked to prevent replay."
      : null,
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [getPreview] = usePreviewBuddyResourceMutation();
  const [prepare] = usePrepareBuddyResourceMutation();
  const [getRights] = useRightsBuddyResourceMutation();
  const [grant] = useGrantBuddyResourceMutation();
  const [execute] = useExecuteBuddyResourceMutation();
  const [status] = useLazyStatusBuddyResourceQuery();
  const available =
    spec.availability === "available" &&
    spec.authority_class === "mutating" &&
    spec.confirmation === "user_confirmation_required";
  const blocked = disabled || busy || initial.blocked;
  const operation = async (run: () => Promise<void>) => {
    if (lock.current || blocked) return;
    lock.current = true;
    setBusy(true);
    setError(null);
    try {
      await run();
    } catch {
      setError(
        "The server did not acknowledge this operation. It may be unavailable, stale, permission denied, or the response was lost. No action is replayed. Refresh saved execution status before doing anything else.",
      );
      setConfirmed(false);
      setRightsConfirmed(false);
      setRecoverConfirmed(false);
    } finally {
      lock.current = false;
      setBusy(false);
    }
  };
  const review = () =>
    operation(async () => {
      setPreview(null);
      setPrepared(null);
      setConfirmed(false);
      setRightsConfirmed(false);
      const nextPreview = await getPreview(context).unwrap();
      const nextRights = await getRights(context).unwrap();
      const nextPrepared = await prepare(context).unwrap();
      setPreview(nextPreview);
      setRights(nextRights);
      setPrepared(nextPrepared);
    });
  const apply = (recover: boolean) =>
    operation(async () => {
      if (!available || !spec.capabilities.can_execute) return;
      if (
        recover
          ? !execution?.recovery?.available || !recoverConfirmed
          : !prepared ||
            !preview ||
            !confirmed ||
            (rights !== null && !rightsConfirmed) ||
            tracking !== null ||
            execution !== null
      )
        return;
      const original = recover ? execution : null;
      const hash = original?.spec_hash ?? prepared?.spec_hash;
      if (!hash) return;
      const recoveryId = original?.execution_id ?? null;
      const authority = await grant({
        ...context,
        spec_hash: hash,
        native_rights: recover ? null : rights,
        recovery_execution_id: recoveryId,
      }).unwrap();
      const executionId = uuidv4();
      const pending = {
        execution_id: executionId,
        idempotency_key: executionId,
        spec_hash: hash,
        recovery_execution_id: recoveryId,
        recovery_id: original?.recovery?.recovery_id ?? null,
      };
      // Persist before sending. Never clear this marker or automatically retry a mutation.
      localStorage.setItem(storageKey, JSON.stringify(pending));
      setTracking(pending);
      setExecution(null);
      setConfirmed(false);
      setRecoverConfirmed(false);
      const result = await execute({
        ...context,
        ...pending,
        recovery_execution_id: recoveryId ?? undefined,
        recovery_id: pending.recovery_id ?? undefined,
        grant_id: authority.grant_id,
        idempotency_key: pending.execution_id,
      }).unwrap();
      setExecution(result.execution);
      setNotice(
        result.disposition === "Duplicate"
          ? "Existing execution returned; no new action was started."
          : "Execution receipt received. Check the observed effects below; a receipt alone is not proof of success.",
      );
    });
  const refresh = () =>
    operation(async () => {
      const saved: Tracking | null =
        tracking ??
        (execution
          ? {
              execution_id: execution.execution_id,
              spec_hash: execution.spec_hash,
              idempotency_key: execution.idempotency_key,
              recovery_execution_id: null,
              recovery_id: null,
            }
          : null);
      if (!saved) return;
      setExecution(
        await status(
          {
            ...context,
            ...saved,
            recovery_execution_id: saved.recovery_execution_id ?? undefined,
            recovery_id: saved.recovery_id ?? undefined,
          },
          false,
        ).unwrap(),
      );
      setRecoverConfirmed(false);
    });
  const canRecover =
    execution?.recovery?.available === true &&
    spec.capabilities.can_recover &&
    tracking?.recovery_execution_id == null;
  return (
    <section className={styles.action} aria-label={label}>
      <h4>{label}</h4>
      <p>
        Availability: {humanizeIdentifier(spec.availability)}. Actions require
        server rollout and explicit consent; nothing runs automatically.
      </p>
      <dl>
        <dt>Project</dt>
        <dd>{spec.project.canonical_root}</dd>
        <dt>Exact target</dt>
        <dd>{spec.resource.resource_id}</dd>
        <dt>Expected version</dt>
        <dd>{spec.expected_version}</dd>
        <dt>Authority</dt>
        <dd>{humanizeIdentifier(spec.authority_class)}</dd>
      </dl>
      <Button
        size="sm"
        disabled={
          blocked ||
          !available ||
          !spec.capabilities.can_preview ||
          tracking !== null ||
          execution !== null
        }
        onClick={() => void review()}
      >
        Preview exact change
      </Button>
      {preview && prepared && (
        <>
          <p>Proposed version: {preview.proposed_version}</p>
          <pre className={styles.preview}>
            {preview.diff ??
              "The server supplied no textual diff. Review the exact scope and versions above."}
          </pre>
          {preview.reload_needed && (
            <p>
              Disk changes may require a separate native reload. This control
              does not activate or reload them.
            </p>
          )}
          <h5>Separate native rights</h5>
          <pre className={styles.preview}>
            {rights === null
              ? "No additional native rights requested by the server."
              : JSON.stringify(rights, null, 2)}
          </pre>
          {rights !== null && (
            <label>
              <input
                type="checkbox"
                checked={rightsConfirmed}
                disabled={blocked || tracking !== null}
                onChange={(event) => setRightsConfirmed(event.target.checked)}
              />
              I authorize exactly these native command, tools, or install
              rights.
            </label>
          )}
          <label>
            <input
              type="checkbox"
              checked={confirmed}
              disabled={blocked || tracking !== null}
              onChange={(event) => setConfirmed(event.target.checked)}
            />
            I confirm this exact scope, version, and change.
          </label>
          <Button
            size="sm"
            disabled={
              blocked ||
              !available ||
              !spec.capabilities.can_execute ||
              !confirmed ||
              (rights !== null && !rightsConfirmed) ||
              tracking !== null ||
              execution !== null
            }
            onClick={() => void apply(false)}
          >
            Grant and execute once
          </Button>
        </>
      )}
      {(tracking || execution) && (
        <>
          <p>
            Tracked execution:{" "}
            {tracking?.execution_id ?? execution?.execution_id}. This action
            cannot be replayed from this control.
          </p>
          <Button size="sm" disabled={blocked} onClick={() => void refresh()}>
            Refresh saved execution
          </Button>
        </>
      )}
      {execution && (
        <div role="status">
          <p>Observed result: {humanizeIdentifier(execution.status)}</p>
          <ul>
            {execution.effects.map((effect, index) => (
              <li key={index}>
                {effect.resource.resource_id}:{" "}
                {humanizeIdentifier(effect.state)}
              </li>
            ))}
          </ul>
          <ul>
            {execution.verification_evidence.map((item, index) => (
              <li key={index}>{item}</li>
            ))}
          </ul>
          <p>{execution.ended_at ?? "No terminal observation recorded."}</p>
        </div>
      )}
      {canRecover && execution && (
        <>
          <p>
            Recovery applies only to original execution {execution.execution_id}
            , at post-version {execution.recovery?.expected_post_version}. The
            server must revalidate availability. Recovery is not a retry.
          </p>
          <label>
            <input
              type="checkbox"
              checked={recoverConfirmed}
              disabled={blocked}
              onChange={(event) => setRecoverConfirmed(event.target.checked)}
            />
            I authorize recovery of this original execution only.
          </label>
          <Button
            size="sm"
            disabled={blocked || !available || !recoverConfirmed}
            onClick={() => void apply(true)}
          >
            Grant original-execution recovery
          </Button>
        </>
      )}
      {notice && <p role="status">{notice}</p>}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
