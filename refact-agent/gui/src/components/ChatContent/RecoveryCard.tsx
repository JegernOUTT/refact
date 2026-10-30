import { useCallback, useEffect, useState } from "react";
import { Button, FieldText } from "../ui";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useChatCommandRequest } from "../../hooks/useChatCommandRequest";
import { useChatActions } from "../../hooks/useChatActions";
import type { RootState } from "../../app/store";
import type { RecoveryRecord } from "../../services/refact/chatSubscription";
import styles from "./ErrorMessage.module.css";
import { ModelSelector } from "../Chat/ModelSelector";
import { humanizeIdentifier } from "../../utils/displayNames";
import {
  selectRecoveryById,
  selectThreadById,
} from "../../features/Chat/Thread/selectors";

export function RecoveryCard({
  chatId,
  recovery,
}: {
  chatId: string;
  recovery: RecoveryRecord;
}) {
  const [now, setNow] = useState(Date.now());
  const [count, setCount] = useState(1);
  const [delay, setDelay] = useState(recovery.policy.delay_ms / 1000);
  const [showOptions, setShowOptions] = useState(false);
  const [model, setModel] = useState("");
  const [timeout, setTimeoutValue] = useState(
    recovery.policy.total_timeout_ms / 1000,
  );
  const [stopping, setStopping] = useState(false);
  const [stopError, setStopError] = useState<string | null>(null);
  const matches = useCallback(
    (state: RootState) =>
      selectRecoveryById(state, chatId)?.run_id === recovery.run_id,
    [chatId, recovery.run_id],
  );
  const request = useChatCommandRequest(chatId, recovery.run_id, matches);
  const { abort, controlGoal } = useChatActions(chatId);
  const goal = useAppSelector((state) => selectThreadById(state, chatId)?.goal);
  const goalHeld =
    goal?.status === "paused" ||
    (goal?.status === "stopped" && goal.stop_reason !== "manual_abort");
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    setStopping(false);
    setStopError(null);
  }, [recovery.run_id]);
  const active =
    recovery.state === "waiting" || recovery.state === "attempting";
  const cooldown = (recovery.retry_not_before_ms ?? 0) > now;
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
              delay_ms: delay * 1000,
              total_timeout_ms: timeout * 1000,
            },
          } as const);
    void request.run(command, (state) => {
      const current = selectRecoveryById(state, chatId);
      return (
        !!current &&
        (current.run_id !== recovery.run_id ||
          current.revision > recovery.revision)
      );
    });
  };
  if (recovery.state === "succeeded") {
    return (
      <details className={styles.callout}>
        <summary>Recovered after {recovery.retries_used} new requests</summary>
        <div>
          {recovery.latest_error.title}: {recovery.latest_error.explanation}
        </div>
        {recovery.attempts.map((attempt, index) => (
          <div key={`${attempt.at_ms}-${index}`}>
            Retry {attempt.retry_number}: {humanizeIdentifier(attempt.outcome)}
          </div>
        ))}
      </details>
    );
  }
  return (
    <section className={styles.callout} aria-label="Recovery status">
      <strong>{recovery.latest_error.title}</strong>
      <div>{recovery.latest_error.explanation}</div>
      <div>
        {recovery.state === "waiting"
          ? `Waiting to retry${
              recovery.next_retry_at_ms
                ? ` in ${Math.max(
                    0,
                    Math.ceil((recovery.next_retry_at_ms - now) / 1000),
                  )}s`
                : ""
            }`
          : humanizeIdentifier(recovery.state)}
      </div>
      <div>
        New requests: {recovery.retries_used} / {recovery.policy.max_retries}
      </div>
      <div>Policy source: {humanizeIdentifier(recovery.policy_source)}</div>
      {active && recovery.deadline_at_ms && (
        <div>
          Time remaining:{" "}
          {Math.max(0, Math.ceil((recovery.deadline_at_ms - now) / 1000))}s
        </div>
      )}
      {recovery.terminal_reason && (
        <div>Reason: {humanizeIdentifier(recovery.terminal_reason)}</div>
      )}
      {cooldown && active && (
        <div>
          Provider cooldown until{" "}
          {new Date(recovery.retry_not_before_ms ?? 0).toLocaleTimeString()}
        </div>
      )}
      {goal && (
        <div>
          {goalHeld
            ? "The goal is paused or stopped. Resume it explicitly before retrying."
            : "Automatic goal continuation is held while recovery needs attention."}
        </div>
      )}
      {goalHeld && (
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
      )}
      <details>
        <summary>Details and attempt history</summary>
        <div>{recovery.latest_error.raw_error}</div>
        {recovery.attempts.map((attempt, index) => (
          <div key={`${attempt.at_ms}-${index}`}>
            {attempt.retry_number === 0
              ? "Original request"
              : `Retry ${attempt.retry_number}`}
            : {humanizeIdentifier(attempt.outcome)}
            {attempt.error ? ` — ${attempt.error.title}` : ""}
          </div>
        ))}
      </details>
      {request.error && <div role="alert">{request.error}</div>}
      {stopError && <div role="alert">{stopError}</div>}
      {request.status && <div>{request.status}</div>}
      {request.uncertain && (
        <Button size="sm" onClick={() => void request.checkStatus()}>
          Check status
        </Button>
      )}
      {active ? (
        <>
          <Button
            size="sm"
            disabled={stopping}
            onClick={() => {
              setStopping(true);
              setStopError(null);
              void abort().catch(() => {
                setStopping(false);
                setStopError(
                  "Could not request Stop. Check the connection and try again.",
                );
              });
            }}
          >
            {stopping ? "Stop requested…" : "Stop"}
          </Button>
          {recovery.state === "waiting" && (
            <Button
              size="sm"
              disabled={cooldown || request.blocked || stopping}
              onClick={() => apply("recovery_retry_now")}
            >
              Retry now
            </Button>
          )}
          <Button
            size="sm"
            disabled={stopping}
            onClick={() => {
              setShowOptions(true);
              setStopping(true);
              void abort().catch(() => {
                setStopping(false);
                setStopError(
                  "Could not request Stop. Check the connection and try again.",
                );
              });
            }}
          >
            Stop to change model
          </Button>
        </>
      ) : (
        <>
          <Button
            size="sm"
            disabled={
              request.blocked ||
              goalHeld ||
              !Number.isSafeInteger(count) ||
              !Number.isSafeInteger(delay * 1000) ||
              !Number.isSafeInteger(timeout * 1000) ||
              count < 1 ||
              count > 100 ||
              delay < 0 ||
              delay > 3600 ||
              timeout < 1 ||
              timeout > 86400
            }
            onClick={() => apply("recovery_retry")}
          >
            {model ? `Retry with ${model}` : "Retry"}
          </Button>
          <Button size="sm" onClick={() => setShowOptions(!showOptions)}>
            Retry options
          </Button>
          {showOptions && (
            <div>
              <label>
                New requests{" "}
                <FieldText
                  type="number"
                  min="1"
                  max="100"
                  value={String(count)}
                  onChange={(value) => setCount(Number(value))}
                />
              </label>
              <label>
                Delay between retries (seconds){" "}
                <FieldText
                  type="number"
                  min="0"
                  max="3600"
                  value={String(delay)}
                  onChange={(value) => setDelay(Number(value))}
                />
              </label>
              <div>First request is immediate.</div>
              <label>
                Total time limit (seconds){" "}
                <FieldText
                  type="number"
                  min="1"
                  max="86400"
                  value={String(timeout)}
                  onChange={(value) => setTimeoutValue(Number(value))}
                />
              </label>
              <ModelSelector
                value={model || undefined}
                onValueChange={setModel}
                label="Retry with model:"
              />
            </div>
          )}
        </>
      )}
    </section>
  );
}
