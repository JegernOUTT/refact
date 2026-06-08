import React from "react";
import { Badge } from "../ui";
import { CapabilityIcons } from "../../features/Providers/ProviderForm/ProviderModelsList/components";
import type { ModelCapabilities } from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import {
  formatContextWindow,
  formatPricing,
} from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import type { CapCost } from "../../services/refact";
import styles from "./RichModelSelectItem.module.css";

export type RichModelData = {
  displayName: string;
  pricing?: CapCost;
  nCtx?: number;
  capabilities?: ModelCapabilities;
  isDefault?: boolean;
  isChat2?: boolean;
  isTaskPlannerAgent?: boolean;
  isThinking?: boolean;
  isLight?: boolean;
  isBuddy?: boolean;
};

type RichModelSelectItemProps = RichModelData;

export const RichModelSelectItem: React.FC<RichModelSelectItemProps> = ({
  displayName,
  pricing,
  nCtx,
  capabilities,
  isDefault,
  isChat2,
  isTaskPlannerAgent,
  isThinking,
  isLight,
  isBuddy,
}) => {
  return (
    <div className={styles.root}>
      <div className={styles.header}>
        <span className={styles.name}>{displayName}</span>
        {isDefault && <Badge tone="accent">Default</Badge>}
        {isTaskPlannerAgent && <Badge tone="accent">Task Agent</Badge>}
        {isChat2 && <Badge tone="accent">Chat 2</Badge>}
        {isThinking && <Badge tone="accent">Reasoning</Badge>}
        {isLight && <Badge tone="success">Light</Badge>}
        {isBuddy && <Badge tone="warning">Companion</Badge>}
      </div>

      <div className={styles.meta}>
        {pricing && (
          <span
            className={styles.metaText}
            title={formatPricing(pricing, false)}
          >
            {formatPricing(pricing, true)}
          </span>
        )}
        {nCtx && (
          <span
            className={styles.metaText}
            title={`Context window: ${nCtx.toLocaleString()} tokens`}
          >
            {formatContextWindow(nCtx)}
          </span>
        )}
        {capabilities && <CapabilityIcons capabilities={capabilities} />}
      </div>
    </div>
  );
};
