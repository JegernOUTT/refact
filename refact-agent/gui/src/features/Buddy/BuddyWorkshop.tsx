import React from "react";
import { Text } from "@radix-ui/themes";
import {
  GearIcon,
  CubeIcon,
  EraserIcon,
  ListBulletIcon,
  RocketIcon,
} from "@radix-ui/react-icons";
import { useExecuteBuddyAction } from "./hooks/useExecuteBuddyAction";
import type { BuddyAction } from "./types";
import styles from "./BuddyWorkshop.module.css";

const ICON_SIZE = 15;

const WORKSHOP_ITEMS: {
  label: string;
  icon: React.ReactNode;
  action: BuddyAction;
}[] = [
  {
    label: "Customize",
    icon: <GearIcon width={ICON_SIZE} height={ICON_SIZE} />,
    action: { kind: "open_page", page: { type: "customization" } },
  },
  {
    label: "Models",
    icon: <RocketIcon width={ICON_SIZE} height={ICON_SIZE} />,
    action: { kind: "open_page", page: { type: "default_models" } },
  },
  {
    label: "Memories",
    icon: <EraserIcon width={ICON_SIZE} height={ICON_SIZE} />,
    action: { kind: "open_page", page: { type: "knowledge_graph" } },
  },
  {
    label: "Tasks",
    icon: <ListBulletIcon width={ICON_SIZE} height={ICON_SIZE} />,
    action: { kind: "open_page", page: { type: "tasks_list" } },
  },
  {
    label: "Marketplace",
    icon: <CubeIcon width={ICON_SIZE} height={ICON_SIZE} />,
    action: { kind: "open_page", page: { type: "marketplace_hub" } },
  },
];

/**
 * Bottom NavBar-style toolbar mirroring the dashboard home NavBar
 * (icon-on-top, label-below, full-width row with a top divider). Lives at
 * the bottom of the Buddy home so the actions stay reachable without
 * occupying a card slot in the main grid.
 */
export const BuddyWorkshop: React.FC = () => {
  const executeAction = useExecuteBuddyAction();

  return (
    <nav className={styles.workshop} data-testid="buddy-workshop">
      {WORKSHOP_ITEMS.map((item) => (
        <button
          key={item.label}
          type="button"
          className={styles.navButton}
          aria-label={item.label}
          onClick={() => void executeAction(item.action, null, -1)}
        >
          <span className={styles.icon}>{item.icon}</span>
          <Text size="1" className={styles.label}>
            {item.label}
          </Text>
        </button>
      ))}
    </nav>
  );
};
