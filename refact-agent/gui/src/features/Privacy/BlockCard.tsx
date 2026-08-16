import React from "react";
import { Text } from "@radix-ui/themes";
import { GitBranch, ShieldAlert } from "lucide-react";

import { Button } from "../../components/ui";
import type { PrivacyFileRecord } from "../../services/refact/privacy";
import styles from "./PrivacyChat.module.css";

type BlockCardProps = {
  model: string;
  step: number;
  blockedFiles: PrivacyFileRecord[];
  onSwitchModel: () => void;
  onBranchCleanChat: () => void;
};

export const BlockCard: React.FC<BlockCardProps> = ({
  model,
  step,
  blockedFiles,
  onSwitchModel,
  onBranchCleanChat,
}) => {
  return (
    <section className={styles.card} data-testid="privacy-block-card">
      <div className={styles.cardHeader}>
        <ShieldAlert className={styles.cardIcon} aria-hidden="true" />
        <div className={styles.cardCopy}>
          <Text as="div" size="2" weight="medium">
            This model cannot receive this step
          </Text>
          <Text className={styles.cardDescription} as="div" size="2">
            {model} cannot receive {blockedFiles.length}{" "}
            {blockedFiles.length === 1 ? "guarded record" : "guarded records"}
            {blockedFiles[0] ? `, including ${blockedFiles[0].path}` : ""}.
          </Text>
        </div>
      </div>
      <div className={styles.actions}>
        <Button size="sm" variant="soft" onClick={onSwitchModel}>
          Switch to an allowed model
        </Button>
        <Button
          size="sm"
          variant="plain"
          leftIcon={GitBranch}
          onClick={onBranchCleanChat}
        >
          Branch clean chat from before step {step}
        </Button>
      </div>
    </section>
  );
};
