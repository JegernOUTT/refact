import { useEffect, useId, useRef, useState } from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import { v4 as uuidv4 } from "uuid";
import { ShieldAlert, ShieldCheck } from "lucide-react";
import { Button, Icon, Surface, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import {
  useChangeBuddySupervisionMutation,
  useGetBuddySupervisionContextQuery,
  useGetBuddySupervisionStatusQuery,
  buildSupervisionGrant,
  buildGoalIntake,
  buildSteeringNotes,
  buildControl,
  DEFAULT_ENGINEERING_TOOLS,
  describeScope,
  parseTargetFiles,
  selectEngineeringTools,
  MAX_STEERING_NOTES,
  MAX_TARGET_FILES,
  SUPERVISED_ENGINEERING_TOOLS,
  SUPERVISED_READ_TOOLS,
  type ControlKind,
  type GrantedScope,
  type NativePause,
  type SupervisionContext,
  type SupervisionMode,
  type SupervisionScope,
} from "../../services/refact/buddySupervision";
import { humanizeIdentifier } from "../../utils/displayNames";
import type { Mandate } from "./operatorTypes";
import { BuddySectionHeader } from "./BuddySectionHeader";
import styles from "./BuddySupervision.module.css";

const SUPERVISION_POLLING_INTERVAL_MS = 5000;

function pauseSummary(pause: NativePause | null): string {
  if (!pause)
    return "Native worker pause: not reported (no live worker observed).";
  const state = pause.quiescent
    ? "parked; no further model or tool dispatch until resumed or cancelled"
    : pause.requested
      ? "requested; in-flight work is still finishing, not yet parked"
      : "not requested";
  const tool = pause.running_tool
    ? ` Running tool: ${pause.running_tool} (${
        pause.running_tool_cancelable
          ? "cancelable"
          : "not interruptible; cancellation only prevents the next dispatch"
      }).`
    : "";
  return `Native worker pause: ${state}.${tool} Queued native notes: ${pause.queued_native_notes}.`;
}

function blockedReason(
  context: SupervisionContext,
  fresh: boolean,
  visible: boolean,
): string {
  if (!visible)
    return "Mandate controls are paused while this page is in the background.";
  if (!fresh) return "Checking the latest supervision status for this project.";
  if (!context.features.storage)
    return "Buddy storage is unavailable, so this project cannot save a mandate.";
  if (!context.authenticated_owner)
    return "Granting a mandate needs this project's signed-in owner.";
  if (!context.project || !context.worker)
    return "A project and worker must be available before a mandate can be granted.";
  return "";
}

function Controls({
  context,
  identity,
  fresh,
}: {
  context: SupervisionContext;
  identity: string;
  fresh: boolean;
}) {
  const visible = useIsDocumentVisible();
  const [change] = useChangeBuddySupervisionMutation();
  const [model, setModel] = useState("");
  const [turns, setTurns] = useState("10");
  const [tokens, setTokens] = useState("20000");
  const [minutes, setMinutes] = useState("10");
  const [cost, setCost] = useState("");
  const [expiry, setExpiry] = useState("");
  const [mode, setMode] = useState<SupervisionMode>("tool_free");
  const [tools, setTools] = useState<string[]>([...DEFAULT_ENGINEERING_TOOLS]);
  const [targets, setTargets] = useState("");
  const [steer, setSteer] = useState(false);
  const [notes, setNotes] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [formOpen, setFormOpen] = useState(false);
  const [mandate, setMandate] = useState<Mandate | null>(null);
  const [scope, setScope] = useState<GrantedScope | null>(null);
  const [id, setId] = useState("");
  const [lookup, setLookup] = useState("");
  const [observe, setObserve] = useState(false);
  const [content, setContent] = useState("");
  const [criteria, setCriteria] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const reasonId = useId();
  const lock = useRef(false);
  const controlKeys = useRef(new Map<string, string>());
  const project = context.project;
  const args =
    project && context.worker && id
      ? { endpoint_identity: identity, project, worker: context.worker, id }
      : null;
  const status = useGetBuddySupervisionStatusQuery(
    args && observe ? args : skipToken,
    {
      pollingInterval: visible ? SUPERVISION_POLLING_INTERVAL_MS : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  const snapshot =
    !fresh || status.error || status.isFetching
      ? undefined
      : status.currentData;
  const available =
    visible &&
    fresh &&
    !!project &&
    !!context.worker &&
    !!context.authenticated_owner &&
    context.features.supervision &&
    context.features.storage;
  const blocked = !available || busy || uncertain;
  const formDisabled = blocked || !!id;
  const reason = available ? "" : blockedReason(context, fresh, visible);
  useEffect(() => {
    if (status.currentData && !status.error && !status.isFetching)
      setUncertain(false);
  }, [status.currentData, status.error, status.isFetching]);
  const resetConfirmation = () => {
    setConfirmed(false);
    setExpanded(false);
  };
  const editMandate = (edit: () => void) => {
    resetConfirmation();
    edit();
  };
  const formScope = (): SupervisionScope =>
    mode === "engineering"
      ? {
          mode,
          tools: selectEngineeringTools(tools),
          target_files: parseTargetFiles(targets),
        }
      : { mode };
  const formSteering = () => (steer ? buildSteeringNotes(notes) : null);
  let authority = "";
  let authorityError = "";
  try {
    authority = describeScope(formScope(), formSteering());
  } catch (e) {
    authorityError = e instanceof Error ? e.message : "Invalid scope";
  }
  const run = async (request: Parameters<typeof change>[0]) => {
    const stop =
      request.operation === "revoke" || request.operation === "cancel";
    if (lock.current || !available || (uncertain && !stop)) return;
    lock.current = true;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const result = await change(request).unwrap();
      if (result.mandate) setMandate(result.mandate);
      if (result.scope) setScope(result.scope);
      if (result.link) setObserve(true);
      if ("request" in request && "control_id" in request.request)
        controlKeys.current.clear();
      setMessage(
        stop
          ? "Stop acknowledged. Native cancellation may still be pending; inspect status below."
          : "Request acknowledged, not completed. Native owner observations remain authoritative.",
      );
      if (observe) void status.refetch();
    } catch {
      setUncertain(true);
      setError(
        "Request unconfirmed. Do not repeat intake or worker dispatch. Refresh native state or open the native chat. Stop/revoke remains available.",
      );
    } finally {
      lock.current = false;
      setBusy(false);
    }
  };
  const control = (kind: ControlKind, noteId?: string) => {
    if (!args) return;
    try {
      const key = kind === "message" ? `message:${noteId ?? ""}` : kind;
      const control_id = controlKeys.current.get(key) ?? uuidv4();
      controlKeys.current.set(key, control_id);
      if (kind === "message")
        void run({
          ...args,
          operation: "message",
          request: buildControl(kind, control_id, noteId) as {
            control_id: string;
            note_id: string;
          },
        });
      else
        void run({
          ...args,
          operation: kind,
          request: buildControl(kind, control_id),
        });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Invalid control");
    }
  };
  const grant = () => {
    try {
      const nextId = uuidv4();
      const scopeRequest = formScope();
      if (scopeRequest.mode !== "tool_free" && !expanded)
        throw new Error("Confirm the expanded authority separately");
      const request = buildSupervisionGrant(
        context,
        nextId,
        model,
        Number(turns),
        Number(tokens),
        Number(minutes),
        cost ? Number(cost) : null,
        expiry,
        scopeRequest,
        formSteering(),
      );
      if (!available || !confirmed || !context.worker) return;
      setId(nextId);
      void run({
        endpoint_identity: identity,
        project: request.mandate.project,
        worker: context.worker,
        id: nextId,
        operation: "create",
        request,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Invalid mandate");
    }
  };
  const steerable =
    !!snapshot &&
    snapshot.controls_supported &&
    snapshot.allowed_operations.includes("steer_worker") &&
    !snapshot.link.revoked &&
    snapshot.native_worker !== null &&
    snapshot.native_pause !== null;
  const cancelable = steerable;
  const pauseRequested = !!snapshot?.native_pause?.requested;
  const selectedNote = snapshot?.steering_notes.find((n) => n.note_id === note);
  return (
    <div className={styles.controls}>
      {reason && (
        <Text
          id={reasonId}
          as="p"
          size="1"
          role="status"
          className={styles.reason}
        >
          {reason}
        </Text>
      )}
      {!id && (
        <>
          <Button
            variant="ghost"
            size="sm"
            className={!available ? styles.disclosureBlocked : undefined}
            aria-expanded={formOpen}
            aria-describedby={reason ? reasonId : undefined}
            onClick={() => setFormOpen((open) => !open)}
          >
            {formOpen ? "Hide mandate setup" : "Grant a bounded mandate"}
          </Button>
        </>
      )}
      {!id && formOpen && (
        <>
          <Text as="p" size="1" className={styles.note}>
            One bounded worker in its own worktree on{" "}
            {project?.canonical_root ?? "this project"}. Read-only analysis
            unless you grant more below. Never shell, MCP or auto-merge.
          </Text>
          <div
            role="group"
            aria-label="Explicit, expiring mandate"
            className={styles.group}
            data-disabled={formDisabled ? "true" : undefined}
          >
            <label className={styles.field}>
              Exact model ID
              <input
                className={styles.input}
                disabled={formDisabled}
                value={model}
                onChange={(e) => editMandate(() => setModel(e.target.value))}
                placeholder="Provider/model ID"
              />
            </label>
            <label className={styles.field}>
              Maximum turns (1–100)
              <input
                className={styles.input}
                type="number"
                min="1"
                max="100"
                disabled={formDisabled}
                value={turns}
                onChange={(e) => editMandate(() => setTurns(e.target.value))}
              />
            </label>
            <label className={styles.field}>
              Maximum tokens (1–1,000,000)
              <input
                className={styles.input}
                type="number"
                min="1"
                max="1000000"
                disabled={formDisabled}
                value={tokens}
                onChange={(e) => editMandate(() => setTokens(e.target.value))}
              />
            </label>
            <label className={styles.field}>
              Maximum whole minutes (1–1,440)
              <input
                className={styles.input}
                type="number"
                min="1"
                max="1440"
                disabled={formDisabled}
                value={minutes}
                onChange={(e) => editMandate(() => setMinutes(e.target.value))}
              />
            </label>
            <label className={styles.field}>
              Optional maximum cost (whole cents)
              <input
                className={styles.input}
                type="number"
                min="1"
                disabled={formDisabled}
                value={cost}
                onChange={(e) => editMandate(() => setCost(e.target.value))}
              />
            </label>
            <label className={styles.field}>
              Mandate expires (local time, within 24 hours)
              <input
                className={styles.input}
                type="datetime-local"
                disabled={formDisabled}
                value={expiry}
                onChange={(e) => editMandate(() => setExpiry(e.target.value))}
              />
            </label>
          </div>
          <div
            role="group"
            aria-label="Worker authority"
            className={styles.group}
            data-disabled={formDisabled ? "true" : undefined}
          >
            <label className={styles.field}>
              Worker scope
              <select
                className={styles.select}
                disabled={formDisabled}
                value={mode}
                onChange={(e) =>
                  editMandate(() => setMode(e.target.value as SupervisionMode))
                }
              >
                <option value="tool_free">Tool-free analysis</option>
                <option value="read_only">Read-only with read tools</option>
                <option value="engineering">Exact-file engineering</option>
              </select>
            </label>
            {mode === "read_only" && (
              <Text as="p" size="1" className={styles.note}>
                Read tools: {SUPERVISED_READ_TOOLS.join(", ")}.
              </Text>
            )}
            {mode === "engineering" && (
              <>
                <div
                  role="group"
                  aria-label="Native engineering edit tools"
                  className={styles.subgroup}
                >
                  {SUPERVISED_ENGINEERING_TOOLS.map((tool) => (
                    <label key={tool} className={styles.choice}>
                      <input
                        type="checkbox"
                        disabled={formDisabled}
                        checked={tools.includes(tool)}
                        onChange={(e) =>
                          editMandate(() =>
                            setTools(
                              e.target.checked
                                ? [...tools, tool]
                                : tools.filter((t) => t !== tool),
                            ),
                          )
                        }
                      />
                      {tool}
                    </label>
                  ))}
                </div>
                <label className={styles.field}>
                  Exact target files, relative to the project root, one per line
                  (1–{MAX_TARGET_FILES}; no globs, traversal, .git, .refact,
                  target or node_modules)
                  <textarea
                    className={styles.input}
                    disabled={formDisabled}
                    value={targets}
                    onChange={(e) =>
                      editMandate(() => setTargets(e.target.value))
                    }
                  />
                </label>
              </>
            )}
            <label className={styles.choice}>
              <input
                type="checkbox"
                disabled={formDisabled}
                checked={steer}
                onChange={(e) => editMandate(() => setSteer(e.target.checked))}
              />
              Allow native pause/resume and pre-written steering notes (sent by
              note ID only; no free text after grant)
            </label>
            {steer && (
              <label className={styles.field}>
                Pre-authorized steering notes, one per line (up to{" "}
                {MAX_STEERING_NOTES})
                <textarea
                  className={styles.input}
                  disabled={formDisabled}
                  value={notes}
                  onChange={(e) => editMandate(() => setNotes(e.target.value))}
                />
              </label>
            )}
          </div>
          <Text
            as="p"
            size="1"
            role={authorityError ? "alert" : undefined}
            className={authorityError ? styles.alert : styles.note}
          >
            {authorityError ? authorityError : `Exact authority: ${authority}`}
          </Text>
          {mode !== "tool_free" && (
            <label className={styles.choice}>
              <input
                type="checkbox"
                disabled={blocked || !!authorityError}
                checked={expanded}
                onChange={(e) => setExpanded(e.target.checked)}
              />
              I independently confirm this expanded authority beyond tool-free
              analysis
            </label>
          )}
          <label className={styles.choice}>
            <input
              type="checkbox"
              disabled={blocked}
              checked={confirmed}
              onChange={(e) => setConfirmed(e.target.checked)}
            />
            I grant only the exact project, selected model, budgets and
            authority above for goal intake and one worker dispatch
          </label>
          <div className={styles.actions}>
            <Button
              disabled={
                blocked ||
                !confirmed ||
                !!authorityError ||
                (mode !== "tool_free" && !expanded)
              }
              onClick={grant}
            >
              Grant bounded mandate
            </Button>
          </div>
          <div className={styles.actions}>
            <label className={styles.field}>
              Existing mandate ID
              <input
                className={styles.input}
                value={lookup}
                onChange={(e) => setLookup(e.target.value)}
              />
            </label>
            <Button
              variant="ghost"
              disabled={!available || !lookup.trim()}
              onClick={() => {
                setId(lookup.trim());
                setObserve(true);
              }}
            >
              Observe existing mandate
            </Button>
          </div>
        </>
      )}
      {id && (
        <>
          <Text as="p" size="1" className={styles.note}>
            Mandate ID: <code>{id}</code>
          </Text>
          {mandate && (
            <pre className={styles.details}>
              {JSON.stringify({ mandate, granted_scope: scope }, null, 2)}
            </pre>
          )}
          {mandate && !observe && !mandate.revoked_at && (
            <div
              role="group"
              aria-label="Goal intake"
              className={styles.group}
              data-disabled={blocked ? "true" : undefined}
            >
              <Text as="p" size="1" className={styles.note}>
                Creating a goal does not start a worker.
              </Text>
              <label className={styles.field}>
                Objective
                <textarea
                  className={styles.input}
                  disabled={blocked}
                  value={content}
                  onChange={(e) => setContent(e.target.value)}
                />
              </label>
              <label className={styles.field}>
                Explicit success criteria, one per line
                <textarea
                  className={styles.input}
                  disabled={blocked}
                  value={criteria}
                  onChange={(e) => setCriteria(e.target.value)}
                />
              </label>
              <Button
                disabled={blocked}
                onClick={() => {
                  try {
                    const request = buildGoalIntake(content, criteria);
                    if (args)
                      void run({ ...args, operation: "intake", request });
                  } catch (e) {
                    setError(e instanceof Error ? e.message : "Invalid intake");
                  }
                }}
              >
                Create paused native goal
              </Button>
            </div>
          )}
          <div className={styles.actions}>
            <Button
              disabled={!available || busy || !args}
              onClick={() => {
                if (args) void run({ ...args, operation: "revoke" });
              }}
            >
              Stop and revoke mandate
            </Button>
            <Button
              variant="ghost"
              disabled={!available || busy}
              onClick={() => {
                setObserve(true);
                if (observe) void status.refetch();
              }}
            >
              Refresh native supervision status
            </Button>
          </div>
          {status.isFetching && (
            <Text as="p" size="1" role="status" className={styles.note}>
              Refreshing native observations…
            </Text>
          )}
          {Boolean(status.error) && (
            <Text as="p" size="1" role="alert" className={styles.alert}>
              Native status unavailable. State is unknown/offline, never
              completed.
            </Text>
          )}
          {snapshot && (
            <>
              <Text as="p" size="1" className={styles.note}>
                Native goal:{" "}
                {humanizeIdentifier(
                  snapshot.native_goal?.status ?? "unknown/offline",
                )}
                . Worker:{" "}
                {humanizeIdentifier(
                  snapshot.native_worker?.status ?? "unknown/offline",
                )}
                . Worker goal:{" "}
                {humanizeIdentifier(
                  snapshot.native_worker_goal?.status ?? "unknown/offline",
                )}
                .
              </Text>
              <Text as="p" size="1" className={styles.note}>
                {snapshot.coverage}
              </Text>
              <Text as="p" size="1" className={styles.note}>
                Command acknowledgement:{" "}
                {snapshot.link.command_acknowledged ? "queued" : "unconfirmed"}.
                Dispatch attempted:{" "}
                {snapshot.link.dispatch_attempted
                  ? "yes (never replay automatically)"
                  : "no"}
                . Revoked: {snapshot.link.revoked ? "yes" : "no"}.
              </Text>
              {snapshot.link.escalation && (
                <Text as="p" size="1" role="status" className={styles.note}>
                  {snapshot.link.escalation}
                </Text>
              )}
              <Text as="p" size="1" className={styles.note}>
                {snapshot.link.delivery_run_id
                  ? `Native verified delivery reference: ${snapshot.link.delivery_run_id}`
                  : "No verified delivery observed. A worker completion claim is not objective completion."}
              </Text>
              <div className={styles.actions}>
                <Button
                  disabled={
                    blocked ||
                    status.isFetching ||
                    snapshot.link.revoked ||
                    snapshot.link.dispatch_attempted ||
                    snapshot.native_goal?.status !== "paused"
                  }
                  onClick={() => {
                    if (args) void run({ ...args, operation: "spawn" });
                  }}
                >
                  Start one bounded analysis worker
                </Button>
                <Button
                  variant="ghost"
                  disabled={
                    blocked || status.isFetching || snapshot.link.revoked
                  }
                  onClick={() => {
                    if (args) void run({ ...args, operation: "reconcile" });
                  }}
                >
                  Check native verification evidence
                </Button>
              </div>
              <Text as="p" size="1" className={styles.note}>
                {snapshot.controls_supported
                  ? `Native granted operations: ${
                      snapshot.allowed_operations.join(", ") || "none"
                    }.`
                  : "Native worker controls unsupported by this owner; nothing is enabled."}{" "}
                {pauseSummary(snapshot.native_pause)}
                {snapshot.link.pause_requested
                  ? " Owner pause intent recorded."
                  : ""}
              </Text>
              <div className={styles.actions}>
                <Button
                  variant="ghost"
                  disabled={
                    blocked || status.isFetching || !steerable || pauseRequested
                  }
                  onClick={() => control("pause")}
                >
                  Pause native worker
                </Button>
                <Button
                  variant="ghost"
                  disabled={
                    blocked ||
                    status.isFetching ||
                    !steerable ||
                    !pauseRequested
                  }
                  onClick={() => control("resume")}
                >
                  Resume native worker
                </Button>
                <Button
                  disabled={
                    !available || busy || status.isFetching || !cancelable
                  }
                  onClick={() => control("cancel")}
                >
                  Cancel native worker
                </Button>
              </div>
              {snapshot.controls_supported && (
                <label className={styles.field}>
                  Pre-authorized steering note
                  <select
                    className={styles.select}
                    disabled={blocked || !steerable}
                    value={selectedNote ? note : ""}
                    onChange={(e) => setNote(e.target.value)}
                  >
                    <option value="">Select a note</option>
                    {snapshot.steering_notes.map((n) => (
                      <option key={n.note_id} value={n.note_id}>
                        {n.note_id}: {n.text}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <div className={styles.actions}>
                <Button
                  disabled={
                    blocked || status.isFetching || !steerable || !selectedNote
                  }
                  onClick={() => control("message", selectedNote?.note_id)}
                >
                  Send steering note by ID
                </Button>
              </div>
              {snapshot.link.control_history.length > 0 && (
                <pre className={styles.details}>
                  {snapshot.link.control_history
                    .map(
                      (c) =>
                        `${c.kind}${c.note_id ? ` ${c.note_id}` : ""} → ${
                          c.outcome
                        }`,
                    )
                    .join("\n")}
                </pre>
              )}
            </>
          )}
        </>
      )}
      {busy && (
        <Text as="p" size="1" role="status" className={styles.note}>
          Submitting to the native owner…
        </Text>
      )}
      {message && (
        <Text as="p" size="1" role="status" className={styles.note}>
          {message}
        </Text>
      )}
      {error && (
        <Text as="p" size="1" role="alert" className={styles.alert}>
          {error}
        </Text>
      )}
    </div>
  );
}
function Supervision({ identity }: { identity: string }) {
  const visible = useIsDocumentVisible();
  const [halted, setHalted] = useState(false);
  const query = useGetBuddySupervisionContextQuery(
    { endpoint_identity: identity },
    {
      pollingInterval: visible && !halted ? SUPERVISION_POLLING_INTERVAL_MS : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  const context = query.currentData;
  const failed = Boolean(query.error);
  useEffect(() => {
    setHalted(failed);
  }, [failed]);
  if (failed)
    return (
      <Surface
        as="section"
        variant="glass"
        radius="card"
        className={styles.supervision}
        aria-label="Buddy supervision"
      >
        <BuddySectionHeader icon={ShieldCheck} label="Supervision" />
        <div className={styles.errorRow} role="alert">
          <Icon icon={ShieldAlert} size="sm" tone="danger" />
          <Text as="p" size="1" className={styles.errorText}>
            Supervision context could not be loaded. No permission or completion
            is assumed — native approvals and stop controls are unaffected.
          </Text>
          <Button size="sm" onClick={() => void query.refetch()}>
            Retry
          </Button>
        </div>
      </Surface>
    );
  if (!context?.features.supervision) return null;
  return (
    <Surface
      as="section"
      variant="glass"
      radius="card"
      className={styles.supervision}
      aria-label="Buddy supervision"
    >
      <BuddySectionHeader
        icon={ShieldCheck}
        label="Supervision"
        actions={
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void query.refetch()}
          >
            Refresh supervision context
          </Button>
        }
      />
      <Controls
        key={JSON.stringify([
          identity,
          context.project,
          context.worker,
          context.authenticated_owner,
        ])}
        context={context}
        identity={identity}
        fresh={!query.error && !query.isFetching}
      />
    </Surface>
  );
}
export function BuddySupervision() {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <Supervision key={identity} identity={identity} />;
}
