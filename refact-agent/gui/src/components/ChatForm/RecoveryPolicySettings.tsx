import { useCallback, useEffect, useState } from "react";
import { Button, FieldText, FieldSelect } from "../ui";
import { selectThreadById } from "../../features/Chat/Thread/selectors";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useChatCommandRequest } from "../../hooks/useChatCommandRequest";
import type { RecoveryPolicyOverrides } from "../../services/refact/chatSubscription";
import { useGetCoordinationSettingsQuery } from "../../services/refact/performance";
import styles from "./RecoveryPolicySettings.module.css";

export function RecoveryPolicySettings({
  chatId,
  embedded = false,
}: {
  chatId: string;
  embedded?: boolean;
}) {
  const current = useAppSelector(
    (state) => selectThreadById(state, chatId)?.recovery_policy,
  );
  const { data: inherited } = useGetCoordinationSettingsQuery("project");
  const [mode, setMode] = useState<"inherit" | "off" | "transient">(
    current?.mode ?? "inherit",
  );
  const [count, setCount] = useState(current?.max_retries?.toString() ?? "");
  const [delay, setDelay] = useState(
    current?.delay_ms != null ? String(current.delay_ms / 1000) : "",
  );
  const [timeout, setTimeoutValue] = useState(
    current?.total_timeout_ms != null
      ? String(current.total_timeout_ms / 1000)
      : "",
  );
  const currentKey = JSON.stringify(current ?? null);
  const [loadedKey, setLoadedKey] = useState(currentKey);
  const [edited, setEdited] = useState(false);
  useEffect(() => {
    if (edited) return;
    setMode(current?.mode ?? "inherit");
    setCount(current?.max_retries?.toString() ?? "");
    setDelay(current?.delay_ms != null ? String(current.delay_ms / 1000) : "");
    setTimeoutValue(
      current?.total_timeout_ms != null
        ? String(current.total_timeout_ms / 1000)
        : "",
    );
    setLoadedKey(currentKey);
  }, [chatId, current, currentKey, edited]);
  const changedElsewhere = edited && currentKey !== loadedKey;
  const matches = useCallback(() => true, []);
  const request = useChatCommandRequest(chatId, "recovery-policy", matches);
  const validNumber = (value: string, min: number, max: number) =>
    value === "" ||
    (/^\d+$/.test(value) && Number(value) >= min && Number(value) <= max);
  const valid =
    validNumber(count, 0, 100) &&
    validNumber(delay, 0, 3600) &&
    validNumber(timeout, 1, 86400);
  const save = () => {
    const policy: RecoveryPolicyOverrides = {
      mode: mode === "inherit" ? null : mode,
      max_retries: count === "" ? null : Number(count),
      delay_ms: delay === "" ? null : Number(delay) * 1000,
      total_timeout_ms: timeout === "" ? null : Number(timeout) * 1000,
    };
    void request
      .run(
        { type: "set_params", patch: { recovery_policy: policy } },
        (state) => {
          const saved = selectThreadById(state, chatId)?.recovery_policy;
          return (
            (saved?.mode ?? null) === policy.mode &&
            (saved?.max_retries ?? null) === policy.max_retries &&
            (saved?.delay_ms ?? null) === policy.delay_ms &&
            (saved?.total_timeout_ms ?? null) === policy.total_timeout_ms
          );
        },
      )
      .then((applied) => {
        if (applied) setEdited(false);
      });
  };
  const effective = inherited?.effective;
  const effectiveMode =
    current?.mode ??
    (effective?.recovery_auto_retry_enabled === true ? "transient" : "off");
  const effectiveCount =
    current?.max_retries ?? effective?.recovery_max_retries;
  const effectiveDelay = current?.delay_ms ?? effective?.recovery_delay_ms;
  const effectiveTimeout =
    current?.total_timeout_ms ?? effective?.recovery_total_timeout_ms;
  const hasThreadOverrides =
    current != null && Object.values(current).some((value) => value != null);
  const content = (
    <div className={styles.settings}>
      <p>
        Applies to the next run. Blank values inherit project or global
        settings.
      </p>
      {inherited ? (
        <p>
          Effective settings:{" "}
          {effectiveMode === "transient" ? "transient errors" : "off"};{" "}
          {String(effectiveCount)} new requests; {Number(effectiveDelay) / 1000}
          s delay; {Number(effectiveTimeout) / 1000}s limit. Source:{" "}
          {hasThreadOverrides
            ? "thread (with inherited fields)"
            : inherited.inherited.filter((name) => name.startsWith("recovery_"))
                  .length === 4
              ? "global"
              : "project"}
          .
        </p>
      ) : (
        <p>Inherited settings unavailable.</p>
      )}
      <label>
        Automatic recovery{" "}
        <FieldSelect
          value={mode}
          onChange={(value) => {
            if (
              value === "inherit" ||
              value === "off" ||
              value === "transient"
            ) {
              setMode(value);
              setEdited(true);
            }
          }}
          options={[
            { value: "inherit", label: "Inherit" },
            { value: "off", label: "Off" },
            { value: "transient", label: "Transient errors" },
          ]}
        />
      </label>
      <label>
        Maximum new requests{" "}
        <FieldText
          type="number"
          min="0"
          max="100"
          value={count}
          onChange={(value) => {
            setCount(value);
            setEdited(true);
          }}
        />
      </label>
      <label>
        Delay (seconds){" "}
        <FieldText
          type="number"
          min="0"
          max="3600"
          value={delay}
          onChange={(value) => {
            setDelay(value);
            setEdited(true);
          }}
        />
      </label>
      <label>
        Strict total time limit (seconds){" "}
        <FieldText
          type="number"
          min="1"
          max="86400"
          value={timeout}
          onChange={(value) => {
            setTimeoutValue(value);
            setEdited(true);
          }}
        />
      </label>
      {changedElsewhere && (
        <div role="alert">
          This policy changed elsewhere. Reload the saved values before saving.
          <Button size="sm" onClick={() => setEdited(false)}>
            Reload saved policy
          </Button>
        </div>
      )}
      <Button
        size="sm"
        disabled={!valid || request.blocked || changedElsewhere || !edited}
        onClick={save}
      >
        Save policy
      </Button>
      {request.error && <div role="alert">{request.error}</div>}
      {request.status && <div>{request.status}</div>}
      {request.uncertain && (
        <Button size="sm" onClick={() => void request.checkStatus()}>
          Check status
        </Button>
      )}
    </div>
  );
  return embedded ? (
    content
  ) : (
    <details>
      <summary>Thread recovery policy</summary>
      {content}
    </details>
  );
}
