import { Box, Flex, Text } from "@radix-ui/themes";

import type { ActionabilityDiagnostics } from "../../../services/refact/browser";
import { Badge, StatusDot } from "../../ui";
import { AnimatedCollapsible } from "../shared/AnimatedCollapsible";
import styles from "./ActionabilityLog.module.css";

type StateKey =
  | "attached"
  | "visible"
  | "stable"
  | "enabled"
  | "editable"
  | "receives_events";

const STATE_LABELS: readonly [StateKey, string][] = [
  ["attached", "Attached"],
  ["visible", "Visible"],
  ["stable", "Stable"],
  ["enabled", "Enabled"],
  ["editable", "Editable"],
  ["receives_events", "Receives events"],
];

export interface ActionabilityLogProps {
  diagnostics?: Partial<ActionabilityDiagnostics> | null;
  failed: boolean;
  retryCount?: number;
  stepIndex?: number;
}

function stateResult(value: boolean | undefined) {
  if (value === true) {
    return {
      label: "Pass",
      status: "success" as const,
      tone: "success" as const,
    };
  }
  if (value === false) {
    return { label: "Fail", status: "error" as const, tone: "danger" as const };
  }
  return {
    label: "Not checked",
    status: "idle" as const,
    tone: "muted" as const,
  };
}

function hasCompletePayload(
  diagnostics: Partial<ActionabilityDiagnostics> | null | undefined,
): diagnostics is ActionabilityDiagnostics {
  return (
    diagnostics != null &&
    Array.isArray(diagnostics.call_log) &&
    diagnostics.call_log.every((entry) => typeof entry === "string") &&
    typeof diagnostics.timed_out === "boolean"
  );
}

export function ActionabilityLog({
  diagnostics,
  failed,
  retryCount,
  stepIndex,
}: ActionabilityLogProps) {
  if (!hasCompletePayload(diagnostics) || diagnostics.call_log.length === 0) {
    return null;
  }

  const title =
    stepIndex == null ? "Actionability" : `Step ${stepIndex + 1} actionability`;
  const status = failed ? "error" : "success";

  return (
    <AnimatedCollapsible
      className={styles.log}
      data-failed={failed}
      data-testid="actionability-log"
      defaultOpen={failed}
      header={title}
      status={status}
      variant="compact"
    >
      <Flex align="center" gap="2" wrap="wrap" className={styles.meta}>
        <Badge tone={diagnostics.timed_out ? "danger" : "muted"} size="xs">
          {diagnostics.timed_out ? "Timed out" : "Completed"}
        </Badge>
        {diagnostics.elapsed_ms != null && (
          <Text size="1">{diagnostics.elapsed_ms} ms elapsed</Text>
        )}
        {diagnostics.attempts != null && (
          <Text size="1">
            {diagnostics.attempts} attempt
            {diagnostics.attempts === 1 ? "" : "s"}
          </Text>
        )}
        {retryCount != null && (
          <Text size="1">
            {retryCount} {retryCount === 1 ? "retry" : "retries"}
          </Text>
        )}
      </Flex>

      <Flex gap="1" wrap="wrap" className={styles.states}>
        {STATE_LABELS.map(([key, label]) => {
          const result = stateResult(diagnostics[key]);
          return (
            <Badge
              aria-label={`${label}: ${result.label}`}
              data-result={
                diagnostics[key] == null
                  ? "not-checked"
                  : diagnostics[key]
                    ? "pass"
                    : "fail"
              }
              data-testid={`actionability-state-${key.replace("_", "-")}`}
              key={key}
              size="xs"
              tone={result.tone}
              variant="outline"
            >
              <StatusDot aria-hidden size="small" status={result.status} />
              <span>{label}</span>
              <span className={styles.stateResult}>{result.label}</span>
            </Badge>
          );
        })}
      </Flex>

      {diagnostics.intercepting_element && (
        <Box className={styles.interceptingElement}>
          <Text as="div" className={styles.interceptingLabel} size="1">
            Intercepting element
          </Text>
          <code>{diagnostics.intercepting_element}</code>
        </Box>
      )}

      <Box className={styles.scrollRegion}>
        <ol aria-label="Actionability call log" className={styles.timeline}>
          {diagnostics.call_log.map((entry, index) => {
            const isFinal = index === diagnostics.call_log.length - 1;
            return (
              <li
                className={styles.timelineItem}
                data-final={isFinal}
                key={`${index}-${entry}`}
              >
                <span aria-hidden className={styles.timelineMarker} />
                <span>{entry}</span>
              </li>
            );
          })}
        </ol>
      </Box>
    </AnimatedCollapsible>
  );
}
