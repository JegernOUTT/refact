import React, { useCallback } from "react";
import { BellOff, Wrench } from "lucide-react";
import { Button } from "../../components/ui";
import { CopyButton } from "../BugReport/CopyButton";
import { useUnmuteBuddyRuleMutation } from "../../services/refact/buddy";
import {
  BuddyEmptyState,
  BuddyRow,
  BuddySection,
  BuddyTimestamp,
  buddyRuleLabel,
} from "./ui";
import type { BuddyStorageMetadata, ChatReactionDebug } from "./types";
import { formatChatReactionStatus } from "./BuddySettingsPanelLabels";
import styles from "./BuddySettingsPanel.module.css";

export const BuddyMutedRules: React.FC<{ rules: string[]; name: string }> = ({
  rules,
  name,
}) => {
  const [unmuteRule] = useUnmuteBuddyRuleMutation();
  const handleUnmute = useCallback(
    async (ruleKey: string) => {
      try {
        await unmuteRule({ rule_key: ruleKey }).unwrap();
      } catch {
        return;
      }
    },
    [unmuteRule],
  );
  return (
    <BuddySection
      title="Muted rules"
      icon={BellOff}
      count={rules.length}
      testId="buddy-muted-rules"
    >
      {rules.length === 0 ? (
        <BuddyEmptyState
          compact
          icon={BellOff}
          title="No rules muted."
          hint={`Rules you silence on ${name}'s cards show up here.`}
        />
      ) : (
        <div className={styles.rowList}>
          {rules.map((ruleKey) => {
            const label = buddyRuleLabel(ruleKey);
            return (
              <BuddyRow
                key={ruleKey}
                density="compact"
                title={label}
                actions={
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-label={`Unmute ${label}`}
                    onClick={() => void handleUnmute(ruleKey)}
                  >
                    Unmute
                  </Button>
                }
              />
            );
          })}
        </div>
      )}
    </BuddySection>
  );
};

const PathRow: React.FC<{ label: string; value: string }> = ({
  label,
  value,
}) => (
  <div className={styles.pathRow}>
    <span className={styles.pathLabel}>{label}</span>
    <code className={styles.pathValue}>{value}</code>
    <CopyButton text={value} label={`Copy ${label.toLowerCase()}`} />
  </div>
);

export const BuddyAdvancedDiagnostics: React.FC<{
  storage: BuddyStorageMetadata | undefined;
  chatReactionDebug: ChatReactionDebug | null;
  name: string;
}> = ({ storage, chatReactionDebug, name }) => (
  <BuddySection
    title="Advanced"
    icon={Wrench}
    collapsible={{ persistKey: "buddy.settings.advanced", lazy: true }}
    testId="buddy-storage-diagnostics"
  >
    <div className={styles.diagnostics}>
      {storage ? (
        <>
          <PathRow label={`${name}'s folder`} value={storage.buddy_dir} />
          <PathRow label="Settings file" value={storage.settings_path} />
          <PathRow label="Project root" value={storage.project_root} />
        </>
      ) : (
        <BuddyEmptyState
          compact
          title="Storage details aren't available from this engine."
        />
      )}
      <div
        className={styles.pathRow}
        data-testid="buddy-chat-reaction-diagnostics"
      >
        <span className={styles.pathLabel}>Chat reaction activity</span>
        <span className={styles.note}>
          {formatChatReactionStatus(chatReactionDebug)}
          {chatReactionDebug?.last_emitted_at ? (
            <>
              {" · "}
              <BuddyTimestamp
                value={chatReactionDebug.last_emitted_at}
                prefix="Last sent"
              />
            </>
          ) : null}
        </span>
      </div>
    </div>
  </BuddySection>
);
