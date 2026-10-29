import React, { useEffect, useId, useRef, useState } from "react";
import { skipToken } from "@reduxjs/toolkit/query";
import { v4 as uuidv4 } from "uuid";
import { ShieldCheck } from "lucide-react";
import {
  Button,
  Field,
  FieldText,
  FieldTextarea,
  Select,
  Text,
} from "../../components/ui";
import { Checkbox } from "../../components/Checkbox";
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
import { selectBuddySnapshot } from "./buddySlice";
import {
  BUDDY_TERMS,
  BUDDY_VERBS,
  BuddyDisclosure,
  BuddyErrorCallout,
  BuddySection,
  buddyName,
} from "./ui";
import styles from "./BuddySupervision.module.css";

const SUPERVISION_POLLING_INTERVAL_MS = 5000;

function pauseSummary(pause: NativePause | null): string {
  if (!pause) return "Pause: not reported (no running worker seen).";
  const state = pause.quiescent
    ? "paused; nothing new runs until you resume or cancel"
    : pause.requested
      ? "requested; current work is still finishing"
      : "not requested";
  const tool = pause.running_tool
    ? ` Running tool: ${pause.running_tool} (${
        pause.running_tool_cancelable
          ? "can be cancelled"
          : "can't be interrupted; cancelling only stops the next step"
      }).`
    : "";
  return `Pause: ${state}.${tool} Queued notes: ${pause.queued_native_notes}.`;
}

function unavailableReason(context: SupervisionContext): string {
  if (!context.features.storage)
    return "Buddy storage is unavailable, so this project can't save a permission.";
  if (!context.authenticated_owner)
    return "Only this project's signed-in owner can grant a permission.";
  if (!context.project || !context.worker)
    return "A project and a worker must be available before a permission can be granted.";
  return "";
}

function blockedReason(
  context: SupervisionContext,
  fresh: boolean,
  visible: boolean,
): string {
  if (!visible)
    return "Controls are paused while this page is in the background.";
  if (!fresh) return "Checking the latest status for this project.";
  return unavailableReason(context);
}

const SCOPE_OPTIONS: { value: SupervisionMode; label: string }[] = [
  { value: "tool_free", label: "Think only (no tools)" },
  { value: "read_only", label: "Read files" },
  { value: "engineering", label: "Edit listed files" },
];

function TextField({
  label,
  onChange,
  ...input
}: {
  label: string;
  onChange: (value: string) => void;
} & Omit<React.ComponentProps<typeof FieldText>, "onChange">) {
  const inputId = useId();
  return (
    <Field label={label} htmlFor={inputId} className={styles.field}>
      <FieldText {...input} id={inputId} onChange={onChange} />
    </Field>
  );
}

function TextAreaField({
  label,
  onChange,
  ...input
}: {
  label: string;
  onChange: (value: string) => void;
} & Omit<React.ComponentProps<typeof FieldTextarea>, "onChange">) {
  const inputId = useId();
  return (
    <Field label={label} htmlFor={inputId} className={styles.field}>
      <FieldTextarea {...input} id={inputId} onChange={onChange} />
    </Field>
  );
}

function SelectField({
  label,
  value,
  options,
  disabled,
  placeholder,
  onChange,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  disabled?: boolean;
  placeholder?: string;
  onChange: (value: string) => void;
}) {
  const inputId = useId();
  return (
    <Field label={label} htmlFor={inputId} className={styles.field}>
      <Select
        value={value || undefined}
        disabled={disabled}
        onValueChange={onChange}
      >
        <Select.Trigger
          id={inputId}
          aria-label={label}
          placeholder={placeholder}
          className={styles.select}
        />
        <Select.Content maxWidth="var(--rf-input-max, 360px)">
          {options.map((option) => (
            <Select.Item key={option.value} value={option.value}>
              {option.label}
            </Select.Item>
          ))}
        </Select.Content>
      </Select>
    </Field>
  );
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
  const statusRefetchRef = useRef<() => void>(() => undefined);
  const visible = useIsDocumentVisible(() => statusRefetchRef.current());
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
  statusRefetchRef.current = () => {
    if (observe && !status.error) void status.refetch();
  };
  const snapshot =
    !fresh || Boolean(status.error) || status.isFetching
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
  const unavailable = unavailableReason(context) !== "";
  const buddyLabel = useAppSelector(
    (state) => selectBuddySnapshot(state)?.state.identity.name,
  );
  useEffect(() => {
    if (status.currentData && !status.error && !status.isFetching)
      setUncertain(false);
  }, [
    status.currentData,
    status.error,
    status.isFetching,
    status.fulfilledTimeStamp,
  ]);
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
          ? "Stop received. Cancelling may still be in progress; check the status below."
          : "Request received, not finished yet. The status below is the source of truth.",
      );
      if (observe) void status.refetch();
    } catch {
      setUncertain(true);
      setError(
        "Request unconfirmed. Don't set the goal or start the worker again. Refresh the status or open the chat. Stop and revoke still work.",
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
        throw new Error("Confirm the extra access separately");
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
      setError(e instanceof Error ? e.message : "Invalid limits");
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
  const permissionHint = `${buddyName(
    buddyLabel,
  )} runs one worker in its own worktree on ${
    project?.canonical_root ?? "this project"
  }, within the limits below. It only reads and thinks unless you allow more. It never runs shell commands, uses MCP or merges on its own.`;
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
      {!id && !unavailable && (
        <BuddyDisclosure
          label={`Let ${buddyName(buddyLabel)} work on its own`}
          persistKey="buddy.activity.permissionForm"
          lazy
          testId="buddy-supervision-form"
          contentClassName={styles.controls}
        >
          <Text as="p" size="1" className={styles.note}>
            {permissionHint}
          </Text>
          <div
            role="group"
            aria-label="Limits"
            className={styles.group}
            data-disabled={formDisabled ? "true" : undefined}
          >
            <TextField
              label="Model ID"
              disabled={formDisabled}
              value={model}
              placeholder="provider/model"
              onChange={(value) => editMandate(() => setModel(value))}
            />
            <TextField
              label="Maximum turns (1–100)"
              type="number"
              min="1"
              max="100"
              disabled={formDisabled}
              value={turns}
              onChange={(value) => editMandate(() => setTurns(value))}
            />
            <TextField
              label="Maximum tokens (1–1,000,000)"
              type="number"
              min="1"
              max="1000000"
              disabled={formDisabled}
              value={tokens}
              onChange={(value) => editMandate(() => setTokens(value))}
            />
            <TextField
              label="Maximum minutes (1–1,440)"
              type="number"
              min="1"
              max="1440"
              disabled={formDisabled}
              value={minutes}
              onChange={(value) => editMandate(() => setMinutes(value))}
            />
            <TextField
              label="Maximum cost in cents (optional)"
              type="number"
              min="1"
              disabled={formDisabled}
              value={cost}
              onChange={(value) => editMandate(() => setCost(value))}
            />
            <TextField
              label="Expires (local time, within 24 hours)"
              type="datetime-local"
              disabled={formDisabled}
              value={expiry}
              onChange={(value) => editMandate(() => setExpiry(value))}
            />
          </div>
          <div
            role="group"
            aria-label="Access"
            className={styles.group}
            data-disabled={formDisabled ? "true" : undefined}
          >
            <SelectField
              label="What it may do"
              disabled={formDisabled}
              value={mode}
              options={SCOPE_OPTIONS}
              onChange={(value) =>
                editMandate(() => setMode(value as SupervisionMode))
              }
            />
            {mode === "read_only" && (
              <Text as="p" size="1" className={styles.note}>
                Read tools: {SUPERVISED_READ_TOOLS.join(", ")}.
              </Text>
            )}
            {mode === "engineering" && (
              <>
                <div
                  role="group"
                  aria-label="Edit tools"
                  className={styles.subgroup}
                >
                  {SUPERVISED_ENGINEERING_TOOLS.map((tool) => (
                    <Checkbox
                      key={tool}
                      disabled={formDisabled}
                      checked={tools.includes(tool)}
                      onCheckedChange={(checked) =>
                        editMandate(() =>
                          setTools(
                            checked === true
                              ? [...tools, tool]
                              : tools.filter((t) => t !== tool),
                          ),
                        )
                      }
                    >
                      {tool}
                    </Checkbox>
                  ))}
                </div>
                <TextAreaField
                  label={`Files it may edit, relative to the project root, one per line (1–${MAX_TARGET_FILES}; no globs, .., .git, .refact, target or node_modules)`}
                  disabled={formDisabled}
                  value={targets}
                  onChange={(value) => editMandate(() => setTargets(value))}
                />
              </>
            )}
            <Checkbox
              disabled={formDisabled}
              checked={steer}
              onCheckedChange={(checked) =>
                editMandate(() => setSteer(checked === true))
              }
            >
              Let me pause, resume and send pre-written notes (notes are sent by
              ID; no new text after granting)
            </Checkbox>
            {steer && (
              <TextAreaField
                label={`Pre-written notes, one per line (up to ${MAX_STEERING_NOTES})`}
                disabled={formDisabled}
                value={notes}
                onChange={(value) => editMandate(() => setNotes(value))}
              />
            )}
          </div>
          <Text
            as="p"
            size="1"
            role={authorityError ? "alert" : undefined}
            className={authorityError ? styles.alert : styles.note}
          >
            {authorityError ? authorityError : `Allowed: ${authority}`}
          </Text>
          {mode !== "tool_free" && (
            <Checkbox
              disabled={blocked || !!authorityError}
              checked={expanded}
              onCheckedChange={(checked) => setExpanded(checked === true)}
            >
              I confirm this goes beyond read-only thinking
            </Checkbox>
          )}
          <Checkbox
            disabled={blocked}
            checked={confirmed}
            onCheckedChange={(checked) => setConfirmed(checked === true)}
          >
            I grant only this project, model, limits and access, for one goal
            and one worker run
          </Checkbox>
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
              Grant permission
            </Button>
          </div>
          <div className={styles.actions}>
            <TextField
              label="Existing permission ID"
              value={lookup}
              onChange={setLookup}
            />
            <Button
              variant="ghost"
              disabled={!available || !lookup.trim()}
              onClick={() => {
                setId(lookup.trim());
                setObserve(true);
              }}
            >
              Check existing permission
            </Button>
          </div>
        </BuddyDisclosure>
      )}
      {id && (
        <>
          <BuddyDisclosure label={BUDDY_TERMS.technicalDetails}>
            <pre className={styles.details}>
              {JSON.stringify(
                { permission_id: id, mandate, granted_scope: scope },
                null,
                2,
              )}
            </pre>
          </BuddyDisclosure>
          {mandate && !observe && !mandate.revoked_at && (
            <div
              role="group"
              aria-label="Goal setup"
              className={styles.group}
              data-disabled={blocked ? "true" : undefined}
            >
              <Text as="p" size="1" className={styles.note}>
                Setting a goal doesn&apos;t start any work yet.
              </Text>
              <TextAreaField
                label="Goal"
                disabled={blocked}
                value={content}
                onChange={setContent}
              />
              <TextAreaField
                label="How you'll know it's done, one per line"
                disabled={blocked}
                value={criteria}
                onChange={setCriteria}
              />
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
                Set goal (paused)
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
              Stop and revoke permission
            </Button>
            <Button
              variant="ghost"
              disabled={!available || busy}
              onClick={() => {
                setObserve(true);
                if (observe) void status.refetch();
              }}
            >
              Refresh status
            </Button>
          </div>
          {status.isFetching && (
            <Text as="p" size="1" role="status" className={styles.note}>
              Refreshing status…
            </Text>
          )}
          {Boolean(status.error) && (
            <Text as="p" size="1" role="alert" className={styles.alert}>
              Status unavailable. Treat the state as unknown, not finished.
            </Text>
          )}
          {snapshot && (
            <>
              <Text as="p" size="1" className={styles.note}>
                Goal:{" "}
                {humanizeIdentifier(
                  snapshot.native_goal?.status ?? "unknown/offline",
                )}
                . Worker:{" "}
                {humanizeIdentifier(
                  snapshot.native_worker?.status ?? "unknown/offline",
                )}
                . Worker&apos;s goal:{" "}
                {humanizeIdentifier(
                  snapshot.native_worker_goal?.status ?? "unknown/offline",
                )}
                .
              </Text>
              <Text as="p" size="1" className={styles.note}>
                {snapshot.coverage}
              </Text>
              <Text as="p" size="1" className={styles.note}>
                Request:{" "}
                {snapshot.link.command_acknowledged ? "queued" : "unconfirmed"}.
                Worker started:{" "}
                {snapshot.link.dispatch_attempted
                  ? "yes (never restarted automatically)"
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
                  ? `Verified delivery: ${snapshot.link.delivery_run_id}`
                  : "No verified delivery yet. A worker saying it's done doesn't count as done."}
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
                  Start the worker
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
                  Check verification
                </Button>
              </div>
              <Text as="p" size="1" className={styles.note}>
                {snapshot.controls_supported
                  ? `Allowed controls: ${
                      snapshot.allowed_operations.join(", ") || "none"
                    }.`
                  : "This project doesn't support worker controls; nothing is enabled."}{" "}
                {pauseSummary(snapshot.native_pause)}
                {snapshot.link.pause_requested
                  ? " Your pause request is recorded."
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
                  Pause worker
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
                  Resume worker
                </Button>
                <Button
                  disabled={
                    !available || busy || status.isFetching || !cancelable
                  }
                  onClick={() => control("cancel")}
                >
                  Cancel worker
                </Button>
              </div>
              {snapshot.controls_supported && (
                <SelectField
                  label="Pre-written note"
                  placeholder="Select a note"
                  disabled={blocked || !steerable}
                  value={selectedNote ? note : ""}
                  options={snapshot.steering_notes.map((n) => ({
                    value: n.note_id,
                    label: `${n.note_id}: ${n.text}`,
                  }))}
                  onChange={setNote}
                />
              )}
              <div className={styles.actions}>
                <Button
                  disabled={
                    blocked || status.isFetching || !steerable || !selectedNote
                  }
                  onClick={() => control("message", selectedNote?.note_id)}
                >
                  Send note
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
          Sending…
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
  const [halted, setHalted] = useState(false);
  const refetchRef = useRef<() => void>(() => undefined);
  const visible = useIsDocumentVisible(() => refetchRef.current());
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
  refetchRef.current = () => {
    if (!halted && !query.error) void query.refetch();
  };
  const context = query.currentData;
  const failed = Boolean(query.error);
  useEffect(() => {
    setHalted(failed);
  }, [failed]);
  if (failed)
    return (
      <BuddySection
        title="Supervision"
        icon={ShieldCheck}
        aria-label="Buddy supervision"
      >
        <BuddyErrorCallout
          thing="supervision"
          details="No permission or finished work is assumed. Approvals and stop controls still work."
          onRetry={() => void query.refetch()}
          retrying={query.isFetching}
        />
      </BuddySection>
    );
  if (!context?.features.supervision) return null;
  return (
    <BuddySection
      title="Supervision"
      icon={ShieldCheck}
      aria-label="Buddy supervision"
      action={
        <Button size="sm" variant="ghost" onClick={() => void query.refetch()}>
          {BUDDY_VERBS.refresh}
        </Button>
      }
    >
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
    </BuddySection>
  );
}
export function BuddySupervision() {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <Supervision key={identity} identity={identity} />;
}
