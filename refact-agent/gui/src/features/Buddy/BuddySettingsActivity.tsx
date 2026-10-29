import React, { useMemo } from "react";
import { BarChart3, MessageCircle, MessageCircleOff } from "lucide-react";
import { useGetSpeechDecisionsQuery } from "../../services/refact/buddy";
import { formatCompactNumber } from "./buddyUtils";
import {
  BuddyEmptyState,
  BuddyRow,
  BuddySection,
  BuddyTimestamp,
  buddyWorkflowLabel,
} from "./ui";
import {
  speechIntentLabel,
  speechReasonLabel,
} from "./BuddySettingsPanelLabels";
import type { BuddyWorkflowSummary } from "./types";
import styles from "./BuddySettingsPanel.module.css";

const SPEECH_LIMIT = 20;

function totalTokens(row: BuddyWorkflowSummary): number {
  return (row.tokens_in ?? 0) + (row.tokens_out ?? 0);
}

function hasActivity(row: BuddyWorkflowSummary): boolean {
  return (
    totalTokens(row) > 0 || (row.llm_calls ?? 0) > 0 || (row.outputs ?? 0) > 0
  );
}

export const BuddySpeechDecisions: React.FC<{ name: string }> = ({ name }) => {
  const { data } = useGetSpeechDecisionsQuery(undefined, {
    refetchOnMountOrArgChange: true,
  });
  const decisions = useMemo(
    () => [...(data ?? [])].reverse().slice(0, SPEECH_LIMIT),
    [data],
  );
  return (
    <BuddySection
      title={`Why did ${name} say that?`}
      icon={MessageCircle}
      count={decisions.length}
      description={`The last ${SPEECH_LIMIT} times ${name} decided whether to speak.`}
      testId="buddy-speech-decisions"
    >
      {decisions.length === 0 ? (
        <BuddyEmptyState
          compact
          icon={MessageCircle}
          title={`${name} hasn't said anything yet.`}
        />
      ) : (
        <div className={styles.rowList}>
          {decisions.map((decision) => (
            <BuddyRow
              key={`${decision.at}-${decision.intent ?? ""}-${
                decision.reason
              }-${decision.source}-${decision.preview}`}
              density="compact"
              leading={{
                icon: decision.allowed ? MessageCircle : MessageCircleOff,
                tone: decision.allowed ? "muted" : "warning",
              }}
              title={speechIntentLabel(decision.intent)}
              description={decision.preview}
              meta={[
                speechReasonLabel(decision.reason, decision.allowed),
                <BuddyTimestamp key="at" value={decision.at} />,
              ]}
            />
          ))}
        </div>
      )}
    </BuddySection>
  );
};

export const BuddyJobTelemetry: React.FC<{
  summaries: BuddyWorkflowSummary[];
}> = ({ summaries }) => {
  const rows = useMemo(
    () =>
      summaries
        .filter(hasActivity)
        .sort((a, b) => totalTokens(b) - totalTokens(a)),
    [summaries],
  );
  const idle = summaries.length - rows.length;
  return (
    <BuddySection
      title="Job telemetry"
      icon={BarChart3}
      count={summaries.length}
      collapsible={{ persistKey: "buddy.settings.telemetry", lazy: true }}
      testId="buddy-job-telemetry"
    >
      {rows.length === 0 ? (
        <BuddyEmptyState
          compact
          icon={BarChart3}
          title="No job runs recorded yet."
        />
      ) : (
        <div className={styles.rowList}>
          {rows.map((row) => (
            <BuddyRow
              key={row.workflow_id}
              density="compact"
              title={buddyWorkflowLabel(row.workflow_id)}
              meta={[
                `${formatCompactNumber(row.run_count)} runs`,
                `${formatCompactNumber(totalTokens(row))} tokens`,
                `${formatCompactNumber(row.outputs ?? 0)} outputs`,
                row.last_output_at ? (
                  <BuddyTimestamp
                    key="last"
                    value={row.last_output_at}
                    prefix="Last output"
                  />
                ) : null,
              ]}
            />
          ))}
        </div>
      )}
      {idle > 0 ? (
        <p className={styles.note}>
          {idle === 1
            ? "1 job with no model use is hidden."
            : `${idle} jobs with no model use are hidden.`}
        </p>
      ) : null}
    </BuddySection>
  );
};
