import React from "react";
import { Text } from "@radix-ui/themes";
import { useExecuteBuddyAction } from "./hooks/useExecuteBuddyAction";
import type { BuddyAction } from "./types";
import styles from "./BuddyWorkshop.module.css";

const WORKSHOP_ITEMS: { label: string; icon: string; action: BuddyAction }[] = [
  {
    label: "Open Customization",
    icon: "⚙️",
    action: { kind: "open_page", page: { type: "customization" } },
  },
  {
    label: "Tune Models",
    icon: "🤖",
    action: { kind: "open_page", page: { type: "default_models" } },
  },
  {
    label: "Clean Memories",
    icon: "🧹",
    action: { kind: "open_page", page: { type: "knowledge_graph" } },
  },
  {
    label: "Open Tasks",
    icon: "📋",
    action: { kind: "open_page", page: { type: "tasks_list" } },
  },
  {
    label: "Open Marketplaces",
    icon: "🛒",
    action: { kind: "open_page", page: { type: "marketplace_hub" } },
  },
];

export const BuddyWorkshop: React.FC = () => {
  const executeAction = useExecuteBuddyAction();

  return (
    <div className={styles.workshop} data-testid="buddy-workshop">
      <Text size="1" weight="bold" color="gray" className={styles.label}>
        WORKSHOP
      </Text>
      <div className={styles.grid}>
        {WORKSHOP_ITEMS.map((item) => (
          <button
            key={item.label}
            type="button"
            className={styles.btn}
            aria-label={item.label}
            onClick={() => void executeAction(item.action, null)}
          >
            <span className={styles.btnIcon}>{item.icon}</span>
            <span>{item.label}</span>
          </button>
        ))}
      </div>
    </div>
  );
};
