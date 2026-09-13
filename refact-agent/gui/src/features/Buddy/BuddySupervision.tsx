import { useEffect, useRef, useState } from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import { v4 as uuidv4 } from "uuid";
import { Button } from "../../components/ui";
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
import styles from "./BuddySupervision.module.css";

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
  const lock = useRef(false);
  // Control identities retained across uncertainty so a deliberate re-click
  // presents the same key (native de-duplicates) instead of a second command.
  const controlKeys = useRef(new Map<string, string>());
  const project = context.project;
  const args =
    project && context.worker && id
      ? { endpoint_identity: identity, project, worker: context.worker, id }
      : null;
  const status = useGetBuddySupervisionStatusQuery(
    args && observe ? args : skipToken,
    {
      pollingInterval: visible ? 5000 : 0,
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
  const cancelable =
    !!snapshot &&
    snapshot.controls_supported &&
    snapshot.allowed_operations.includes("steer_worker") &&
    !snapshot.link.revoked &&
    snapshot.native_worker !== null &&
    snapshot.native_pause !== null;
  const pauseRequested = !!snapshot?.native_pause?.requested;
  const selectedNote = snapshot?.steering_notes.find((n) => n.note_id === note);
  return (
    <div className={styles.controls}>
      <p>
        Exact scope: {project?.canonical_root ?? "Project unavailable"} (
        {project?.project_id ?? "unknown"}). One bounded worker in an isolated
        worktree. Default is tool-free, read-only analysis; read_project,
        engineering edit tools, target files and steering controls must be
        granted explicitly below. read_project exposes only its fixed native
        read tools. Never: shell, MCP, auto-merge, automatic answers, permission
        changes or wake/restart guarantees.
      </p>
      <fieldset disabled={blocked || !!id} onChange={resetConfirmation}>
        <legend>Explicit, expiring mandate</legend>
        <label>
          Exact model ID
          <input
            value={model}
            onChange={(e) => editMandate(() => setModel(e.target.value))}
            placeholder="Provider/model ID"
          />
        </label>
        <label>
          Maximum turns (1–100)
          <input
            type="number"
            min="1"
            max="100"
            value={turns}
            onChange={(e) => editMandate(() => setTurns(e.target.value))}
          />
        </label>
        <label>
          Maximum tokens (1–1,000,000)
          <input
            type="number"
            min="1"
            max="1000000"
            value={tokens}
            onChange={(e) => editMandate(() => setTokens(e.target.value))}
          />
        </label>
        <label>
          Maximum whole minutes (1–1,440)
          <input
            type="number"
            min="1"
            max="1440"
            value={minutes}
            onChange={(e) => editMandate(() => setMinutes(e.target.value))}
          />
        </label>
        <label>
          Optional maximum cost (whole cents)
          <input
            type="number"
            min="1"
            value={cost}
            onChange={(e) => editMandate(() => setCost(e.target.value))}
          />
        </label>
        <label>
          Mandate expires (local time, within 24 hours)
          <input
            type="datetime-local"
            value={expiry}
            onChange={(e) => editMandate(() => setExpiry(e.target.value))}
          />
        </label>
      </fieldset>
      <fieldset disabled={blocked || !!id} onChange={resetConfirmation}>
        <legend>Worker authority (default: tool-free)</legend>
        <label>
          Worker scope
          <select
            value={mode}
            onChange={(e) =>
              editMandate(() => setMode(e.target.value as SupervisionMode))
            }
          >
            <option value="tool_free">Tool-free, read-only analysis</option>
            <option value="read_only">
              Read-only with native read tools (
              {SUPERVISED_READ_TOOLS.join(", ")})
            </option>
            <option value="engineering">
              Exact-file engineering (read tools + selected edit tools)
            </option>
          </select>
        </label>
        {mode === "engineering" && (
          <>
            <fieldset>
              <legend>Native engineering edit tools</legend>
              {SUPERVISED_ENGINEERING_TOOLS.map((tool) => (
                <label key={tool}>
                  <input
                    type="checkbox"
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
            </fieldset>
            <label>
              Exact target files, relative to the project root, one per line (1–
              {MAX_TARGET_FILES}; no globs, traversal, .git, .refact, target or
              node_modules)
              <textarea
                value={targets}
                onChange={(e) => editMandate(() => setTargets(e.target.value))}
              />
            </label>
          </>
        )}
        <label>
          <input
            type="checkbox"
            checked={steer}
            onChange={(e) => editMandate(() => setSteer(e.target.checked))}
          />
          Allow native pause/resume and pre-written steering notes (sent by note
          ID only; no free text after grant)
        </label>
        {steer && (
          <label>
            Pre-authorized steering notes, one per line (up to{" "}
            {MAX_STEERING_NOTES})
            <textarea
              value={notes}
              onChange={(e) => editMandate(() => setNotes(e.target.value))}
            />
          </label>
        )}
      </fieldset>
      {!id && (
        <>
          <p role={authorityError ? "alert" : undefined}>
            {authorityError ? authorityError : `Exact authority: ${authority}`}
          </p>
          {mode !== "tool_free" && (
            <label>
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
          <label>
            <input
              type="checkbox"
              disabled={blocked}
              checked={confirmed}
              onChange={(e) => setConfirmed(e.target.checked)}
            />
            I grant only the exact project, selected model, budgets and
            authority above for goal intake and one worker dispatch
          </label>
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
        </>
      )}
      {!id && (
        <div>
          <label>
            Existing mandate ID
            <input value={lookup} onChange={(e) => setLookup(e.target.value)} />
          </label>
          <Button
            disabled={!available || !lookup.trim()}
            onClick={() => {
              setId(lookup.trim());
              setObserve(true);
            }}
          >
            Observe existing mandate
          </Button>
        </div>
      )}
      {id && (
        <>
          <p>
            Mandate ID: <code>{id}</code>
          </p>
          {mandate && (
            <pre className={styles.details}>
              {JSON.stringify({ mandate, granted_scope: scope }, null, 2)}
            </pre>
          )}
          {mandate && !observe && !mandate.revoked_at && (
            <fieldset disabled={blocked}>
              <legend>Goal intake (does not start a worker)</legend>
              <label>
                Objective
                <textarea
                  value={content}
                  onChange={(e) => setContent(e.target.value)}
                />
              </label>
              <label>
                Explicit success criteria, one per line
                <textarea
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
            </fieldset>
          )}
          <Button
            disabled={!available || busy || !args}
            onClick={() => {
              if (args) void run({ ...args, operation: "revoke" });
            }}
          >
            Stop and revoke mandate
          </Button>
          <Button
            disabled={!available || busy}
            onClick={() => {
              setObserve(true);
              if (observe) void status.refetch();
            }}
          >
            Refresh native supervision status
          </Button>
          {status.isFetching && (
            <p role="status">Refreshing native observations…</p>
          )}
          {Boolean(status.error) && (
            <p role="alert">
              Native status unavailable. State is unknown/offline, never
              completed.
            </p>
          )}
          {snapshot && (
            <>
              <p>
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
              </p>
              <p>{snapshot.coverage}</p>
              <p>
                Command acknowledgement:{" "}
                {snapshot.link.command_acknowledged ? "queued" : "unconfirmed"}.
                Dispatch attempted:{" "}
                {snapshot.link.dispatch_attempted
                  ? "yes (never replay automatically)"
                  : "no"}
                . Revoked: {snapshot.link.revoked ? "yes" : "no"}.
              </p>
              {snapshot.link.escalation && (
                <p role="status">{snapshot.link.escalation}</p>
              )}
              <p>
                {snapshot.link.delivery_run_id
                  ? `Native verified delivery reference: ${snapshot.link.delivery_run_id}`
                  : "No verified delivery observed. A worker completion claim is not objective completion."}
              </p>
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
                disabled={blocked || status.isFetching || snapshot.link.revoked}
                onClick={() => {
                  if (args) void run({ ...args, operation: "reconcile" });
                }}
              >
                Check native verification evidence
              </Button>
              <p>
                {snapshot.controls_supported
                  ? `Native granted operations: ${
                      snapshot.allowed_operations.join(", ") || "none"
                    }.`
                  : "Native worker controls unsupported by this owner; nothing is enabled."}{" "}
                {pauseSummary(snapshot.native_pause)}
                {snapshot.link.pause_requested
                  ? " Owner pause intent recorded."
                  : ""}
              </p>
              <Button
                disabled={
                  blocked || status.isFetching || !steerable || pauseRequested
                }
                onClick={() => control("pause")}
              >
                Pause native worker
              </Button>
              <Button
                disabled={
                  blocked || status.isFetching || !steerable || !pauseRequested
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
              {snapshot.controls_supported && (
                <label>
                  Pre-authorized steering note
                  <select
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
              <Button
                disabled={
                  blocked || status.isFetching || !steerable || !selectedNote
                }
                onClick={() => control("message", selectedNote?.note_id)}
              >
                Send steering note by ID
              </Button>
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
      {busy && <p role="status">Submitting to the native owner…</p>}
      {message && <p role="status">{message}</p>}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
function Supervision({ identity }: { identity: string }) {
  const visible = useIsDocumentVisible();
  const query = useGetBuddySupervisionContextQuery(
    { endpoint_identity: identity },
    {
      pollingInterval: visible ? 5000 : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  const context = query.currentData;
  return (
    <section className={styles.supervision} aria-label="Buddy supervision">
      <h3>Buddy supervision</h3>
      <p>
        Explicit mandates only. Native approvals and stop controls do not depend
        on Buddy intelligence.
      </p>
      {query.isFetching && (
        <p role="status">Refreshing rollout and project context…</p>
      )}
      {Boolean(query.error) && (
        <p role="alert">
          Supervision connection or context unavailable. No permission or
          completion is assumed.
        </p>
      )}
      {context && !context.features.supervision && (
        <p role="status">
          Supervision rollout is disabled. Settings have not been changed.
        </p>
      )}
      <Button variant="ghost" onClick={() => void query.refetch()}>
        Refresh supervision context
      </Button>
      {context && (
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
      )}
    </section>
  );
}
export function BuddySupervision() {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <Supervision key={identity} identity={identity} />;
}
