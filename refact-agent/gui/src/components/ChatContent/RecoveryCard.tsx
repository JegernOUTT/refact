import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ChevronDown, Copy, ListTree, Settings2 } from "lucide-react";
import { Button, FieldText, SegmentedControl } from "../ui";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useChatCommandRequest } from "../../hooks/useChatCommandRequest";
import { useChatActions } from "../../hooks/useChatActions";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import type { RootState } from "../../app/store";
import type { RecoveryRecord } from "../../services/refact/chatSubscription";
import type { ErrorMessage } from "../../services/refact/types";
import styles from "./RecoveryCard.module.css";
import { ModelSelector } from "../Chat/ModelSelector";
import { RecoveryPolicySettings } from "../ChatForm/RecoveryPolicySettings";
import { humanizeIdentifier } from "../../utils/displayNames";
import { collectErrorTexts } from "../../services/refact/recoveryPresentation";
import {
  selectRecoveryById,
  selectQueuedItemsById,
  selectThreadById,
} from "../../features/Chat/Thread/selectors";

export function RecoveryDetails({ recovery }: { recovery: RecoveryRecord }) {
  if (recovery.state === "succeeded") return null;
  return (
    <div
      className={styles.history}
      role="region"
      aria-label="Recovery diagnostics"
    >
      <div>Policy source: {humanizeIdentifier(recovery.policy_source)}</div>
      <div>
        Delay: {recovery.policy.delay_ms / 1000}s · Total time limit:{" "}
        {recovery.policy.total_timeout_ms / 1000}s
      </div>
      {recovery.terminal_reason && (
        <div>Reason: {humanizeIdentifier(recovery.terminal_reason)}</div>
      )}
      {recovery.attempts
        .filter((attempt) => attempt.outcome !== "succeeded")
        .map((attempt, index) => (
          <div key={`${attempt.at_ms}-${index}`}>
            {attempt.retry_number === 0
              ? "Original request"
              : `Retry ${attempt.retry_number}`}
            : {humanizeIdentifier(attempt.outcome)}
          </div>
        ))}
    </div>
  );
}

export function RecoveryCard({
  chatId,
  recovery,
  errors = [],
  archived = [],
  submissionKey = "",
}: {
  chatId: string;
  recovery: RecoveryRecord;
  errors?: ErrorMessage[];
  archived?: RecoveryRecord[];
  submissionKey?: string;
}) {
  const [now, setNow] = useState(Date.now());
  const [count, setCount] = useState(1);
  const [delay, setDelay] = useState(recovery.policy.delay_ms / 1000);
  const [timeout, setTimeoutValue] = useState(
    recovery.policy.total_timeout_ms / 1000,
  );
  const [model, setModel] = useState("");
  const [panel, setPanel] = useState<"details" | "options" | null>(null);
  const [scope, setScope] = useState<"next" | "saved">("next");
  const [stopping, setStopping] = useState(false);
  const [stopError, setStopError] = useState<string | null>(null);
  const copyToClipboard = useCopyToClipboard();
  const id = useId();
  const cardRef = useRef<HTMLElement>(null);
  const detailsRef = useRef<HTMLButtonElement>(null);
  const optionsRef = useRef<HTMLButtonElement>(null);
  const optionsRun = useRef(recovery.run_id);
  const matches = useCallback(
    (state: RootState) =>
      selectRecoveryById(state, chatId)?.run_id === recovery.run_id,
    [chatId, recovery.run_id],
  );
  const request = useChatCommandRequest(chatId, "recovery", matches);
  const { abort, controlGoal, cancelQueued } = useChatActions(chatId);
  const queuedItems = useAppSelector((state) =>
    selectQueuedItemsById(state, chatId),
  );
  const queuedRetries = queuedItems.filter(
    (item) =>
      item.command_type === "recovery_retry" ||
      item.command_type === "recovery_retry_now",
  );
  const [cancellingQueued, setCancellingQueued] = useState(false);
  useEffect(() => {
    if (queuedRetries.length === 0) setCancellingQueued(false);
  }, [queuedRetries.length]);
  const goal = useAppSelector((state) => selectThreadById(state, chatId)?.goal);
  const goalHeld =
    goal?.status === "paused" ||
    (goal?.status === "stopped" && goal.stop_reason !== "manual_abort");
  const active =
    recovery.state === "waiting" || recovery.state === "attempting";
  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  useEffect(() => {
    setStopping(false);
    setStopError(null);
  }, [recovery.run_id]);
  useEffect(() => {
    if (optionsRun.current === recovery.run_id) return;
    optionsRun.current = recovery.run_id;
    setCount(1);
    setDelay(recovery.policy.delay_ms / 1000);
    setTimeoutValue(recovery.policy.total_timeout_ms / 1000);
    setModel("");
    setPanel(null);
  }, [
    recovery.run_id,
    recovery.policy.delay_ms,
    recovery.policy.total_timeout_ms,
  ]);
  useEffect(() => {
    const focused = document.activeElement;
    if (
      focused instanceof HTMLElement &&
      focused.closest(`[data-recovery-panel]`) &&
      cardRef.current?.contains(focused)
    )
      focused.blur();
    setPanel(null);
  }, [submissionKey]);
  const cooldown = (recovery.retry_not_before_ms ?? 0) > now;
  const delayMs = Math.round(delay * 1000);
  const timeoutMs = Math.round(timeout * 1000);
  const valid =
    Number.isSafeInteger(count) &&
    count >= 1 &&
    count <= 100 &&
    Number.isSafeInteger(delayMs) &&
    Math.abs(delay * 1000 - delayMs) < 0.000001 &&
    delay >= 0 &&
    delay <= 3600 &&
    Number.isSafeInteger(timeoutMs) &&
    Math.abs(timeout * 1000 - timeoutMs) < 0.000001 &&
    timeout >= 1 &&
    timeout <= 86400;
  const apply = (type: "recovery_retry" | "recovery_retry_now") => {
    const base = {
      run_id: recovery.run_id,
      expected_revision: recovery.revision,
    };
    const command =
      type === "recovery_retry_now"
        ? ({ type, ...base } as const)
        : ({
            type,
            ...base,
            ...(model ? { model } : {}),
            options: {
              mode: "transient" as const,
              max_retries: count,
              delay_ms: delayMs,
              total_timeout_ms: timeoutMs,
            },
          } as const);
    void request.run(command, (state) => {
      const current = selectRecoveryById(state, chatId);
      return (
        !!current &&
        (type === "recovery_retry"
          ? current.run_id !== recovery.run_id &&
            current.manual &&
            current.operation_id === recovery.operation_id
          : current.run_id === recovery.run_id &&
            current.revision > recovery.revision &&
            (current.state === "attempting" ||
              current.state === "succeeded" ||
              current.retries_used > recovery.retries_used ||
              current.attempts.length > recovery.attempts.length))
      );
    });
  };
  const stop = () => {
    setStopping(true);
    setStopError(null);
    void abort().catch(() => {
      setStopping(false);
      setStopError(
        "Could not request Stop. Check the connection and try again.",
      );
    });
  };
  const toggle = (next: "details" | "options") => {
    if (cardRef.current?.contains(document.activeElement)) {
      (next === "details" ? detailsRef : optionsRef).current?.focus();
    }
    setPanel((current) => (current === next ? null : next));
  };
  if (recovery.state === "succeeded") return null;
  const texts = collectErrorTexts(errors, [recovery, ...archived]);
  return (
    <section ref={cardRef} className={styles.card} aria-label="Recovery status">
      <div className={styles.main}>
        <div className={styles.summary}>
          <div className={styles.errors} role="alert">
            {texts.map((text) => (
              <div key={text}>{text}</div>
            ))}
          </div>
          <div className={styles.metadata}>
            <span className={styles.status} role="status">
              {request.uncertain
                ? "Command not confirmed"
                : queuedRetries.length > 0 || request.phase === "queued"
                  ? "Retry queued"
                  : request.pending
                    ? "Awaiting confirmation…"
                    : stopping
                      ? "Stop requested…"
                      : goalHeld
                        ? "Goal paused"
                        : recovery.state === "attempting"
                          ? "Request in progress"
                          : recovery.state === "waiting"
                            ? `${cooldown ? "Provider cooldown" : "Retry"}${
                                recovery.next_retry_at_ms
                                  ? ` · ${Math.max(
                                      0,
                                      Math.ceil(
                                        (recovery.next_retry_at_ms - now) /
                                          1000,
                                      ),
                                    )}s`
                                  : " pending"
                              }`
                            : recovery.terminal_reason
                              ? humanizeIdentifier(recovery.terminal_reason)
                              : humanizeIdentifier(recovery.state)}
            </span>
            <span>
              {recovery.retries_used} / {recovery.policy.max_retries}{" "}
              {recovery.manual ? "new requests used" : "automatic retries used"}
            </span>
          </div>
          {goalHeld && (
            <div className={styles.notice}>
              Resume the goal before retrying. Resume does not send a request.
            </div>
          )}
          {request.error && (
            <div className={styles.notice} role="alert">
              {request.error}
            </div>
          )}
          {stopError && (
            <div className={styles.notice} role="alert">
              {stopError}
            </div>
          )}
          {request.status && (
            <div className={styles.notice}>{request.status}</div>
          )}
          {queuedRetries.length > 0 && !request.status && (
            <div className={styles.notice}>
              Retry accepted and queued. No new request has been sent by this
              queued command.
            </div>
          )}
        </div>
        <div className={styles.actions}>
          {request.uncertain ? (
            <Button size="sm" onClick={() => void request.checkStatus()}>
              Check status
            </Button>
          ) : queuedRetries.length > 0 ||
            (request.phase === "queued" && request.queuedCommandId) ? (
            <Button
              size="sm"
              disabled={cancellingQueued}
              onClick={() => {
                setCancellingQueued(true);
                if (request.queuedCommandId) {
                  void request
                    .cancelQueuedRequest()
                    .finally(() => setCancellingQueued(false));
                } else {
                  const queued = queuedRetries[0];
                  void cancelQueued(queued.client_request_id)
                    .then((confirmed) => {
                      if (!confirmed) {
                        setCancellingQueued(false);
                        setStopError(
                          "Cancellation was not confirmed. The retry remains queued.",
                        );
                      }
                    })
                    .catch(() => {
                      setCancellingQueued(false);
                      setStopError(
                        "Could not cancel the queued retry. Check the connection and try again.",
                      );
                    });
                }
              }}
            >
              {cancellingQueued ? "Cancelling…" : "Cancel queued retry"}
            </Button>
          ) : request.pending ? (
            <Button size="sm" disabled>
              {request.phase === "sending"
                ? "Submitting…"
                : "Awaiting confirmation…"}
            </Button>
          ) : goalHeld ? (
            <Button
              size="sm"
              onClick={() => {
                void controlGoal("resume").catch(() =>
                  setStopError(
                    "Could not resume the goal. Check the connection and try again.",
                  ),
                );
              }}
            >
              Resume goal
            </Button>
          ) : recovery.state === "attempting" ? null : (
            <Button
              size="sm"
              disabled={request.blocked || stopping || cooldown || !valid}
              onClick={() =>
                apply(active ? "recovery_retry_now" : "recovery_retry")
              }
            >
              {active ? "Retry now" : "Retry"}
            </Button>
          )}
          {active && (
            <Button size="sm" variant="soft" disabled={stopping} onClick={stop}>
              {stopping ? "Stop requested…" : "Stop"}
            </Button>
          )}
        </div>
      </div>
      <div className={styles.toggles}>
        <Button
          ref={detailsRef}
          size="sm"
          variant="ghost"
          className={styles.disclosure}
          leftIcon={ListTree}
          rightIcon={ChevronDown}
          aria-expanded={panel === "details"}
          aria-controls={`${id}-details`}
          onClick={() => toggle("details")}
        >
          Details
        </Button>
        <Button
          ref={optionsRef}
          size="sm"
          variant="ghost"
          className={styles.disclosure}
          leftIcon={Settings2}
          rightIcon={ChevronDown}
          aria-expanded={panel === "options"}
          aria-controls={`${id}-options`}
          onClick={() => toggle("options")}
        >
          Options
        </Button>
        <Button
          size="sm"
          variant="ghost"
          leftIcon={Copy}
          aria-label="Copy errors"
          onClick={() => copyToClipboard(texts.join("\n\n"))}
        >
          Copy
        </Button>
      </div>
      <div
        id={`${id}-details`}
        data-recovery-panel
        className={`rf-expand-grid ${styles.panel}`}
        data-open={panel === "details"}
        aria-hidden={panel !== "details"}
        ref={(node) => node?.toggleAttribute("inert", panel !== "details")}
      >
        <div className={styles.panelInner}>
          <RecoveryDetails recovery={recovery} />
          {archived
            .filter(
              (record, index, records) =>
                record.run_id !== recovery.run_id &&
                records.findIndex((other) => other.run_id === record.run_id) ===
                  index,
            )
            .map((record) => (
              <div key={record.run_id} className={styles.history}>
                <strong>
                  {record.manual
                    ? "Previous manual run"
                    : "Previous automatic run"}{" "}
                  · {record.retries_used} / {record.policy.max_retries} used
                </strong>
                <RecoveryDetails recovery={record} />
              </div>
            ))}
        </div>
      </div>
      <div
        id={`${id}-options`}
        data-recovery-panel
        className={`rf-expand-grid ${styles.panel}`}
        data-open={panel === "options"}
        aria-hidden={panel !== "options"}
        ref={(node) => node?.toggleAttribute("inert", panel !== "options")}
      >
        <div className={styles.panelInner}>
          <div className={styles.options}>
            <SegmentedControl
              aria-label="Recovery options scope"
              size="sm"
              layout="content"
              value={scope}
              onValueChange={(value) => {
                if (value === "next" || value === "saved") setScope(value);
              }}
              options={[
                { value: "next", label: "Next retry only" },
                { value: "saved", label: "Saved thread policy" },
              ]}
            />
            <div
              id={`${id}-scope`}
              role="group"
              aria-label={
                scope === "next" ? "Next retry options" : "Saved thread policy"
              }
            >
              {scope === "saved" ? (
                <RecoveryPolicySettings chatId={chatId} embedded />
              ) : (
                <div className={styles.options}>
                  <div className={styles.status}>
                    {active
                      ? "Stop the active recovery before changing the next retry."
                      : "Applies only when you Retry. First request is immediate."}
                  </div>
                  <fieldset
                    disabled={active || request.blocked}
                    className={styles.fields}
                    aria-label="Next retry settings"
                  >
                    <label>
                      New requests
                      <FieldText
                        type="number"
                        min="1"
                        max="100"
                        value={String(count)}
                        onChange={(value) => setCount(Number(value))}
                      />
                    </label>
                    <label>
                      Delay between retries (seconds)
                      <FieldText
                        type="number"
                        min="0"
                        max="3600"
                        step="0.001"
                        value={String(delay)}
                        onChange={(value) => setDelay(Number(value))}
                      />
                    </label>
                    <label>
                      Total time limit (seconds)
                      <FieldText
                        type="number"
                        min="1"
                        max="86400"
                        step="0.001"
                        value={String(timeout)}
                        onChange={(value) => setTimeoutValue(Number(value))}
                      />
                    </label>
                    <ModelSelector
                      value={model}
                      allowUnset
                      unsetLabel="Keep original model"
                      onValueChange={setModel}
                      label="Retry with model:"
                    />
                  </fieldset>
                  {!valid && (
                    <div role="alert">
                      Use 1–100 requests, 0–3600 seconds delay, and 1–86400
                      seconds total time, in whole milliseconds.
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
