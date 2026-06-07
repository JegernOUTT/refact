import React, { useCallback } from "react";
import {
  BarChart3,
  Box,
  Gauge,
  Plug,
  Rocket,
  Settings,
  Timer,
} from "lucide-react";
import { Icon } from "../../../../components/ui";
import { DashboardText as Text } from "../DashboardPrimitives";
import { useAppDispatch } from "../../../../hooks";
import { push, type Page } from "../../../Pages/pagesSlice";
import styles from "./NavBar.module.css";

type NavItem = {
  icon: React.ComponentProps<typeof Icon>["icon"];
  label: string;
  page: Page;
};

const NAV_ITEMS: NavItem[] = [
  {
    icon: BarChart3,
    label: "Stats",
    page: { name: "stats dashboard" },
  },
  {
    icon: Plug,
    label: "Integrations",
    page: { name: "integrations page" },
  },
  {
    icon: Settings,
    label: "Providers",
    page: { name: "providers page" },
  },
  {
    icon: Rocket,
    label: "Modes",
    page: { name: "customization" },
  },
  {
    icon: Timer,
    label: "Scheduler",
    page: { name: "scheduler" },
  },
  {
    icon: Gauge,
    label: "Extensions",
    page: { name: "extensions", tab: "skills" },
  },
  {
    icon: Box,
    label: "Marketplace",
    page: { name: "marketplace hub" },
  },
];

export const NavBar: React.FC = () => {
  const dispatch = useAppDispatch();

  const handleClick = useCallback(
    (page: Page) => {
      dispatch(push(page));
    },
    [dispatch],
  );

  return (
    <nav className={`${styles.nav} rf-enter-rise rf-stagger`}>
      {NAV_ITEMS.map((item) => (
        <button
          key={item.label}
          type="button"
          className={`${styles.navButton} rf-pressable`}
          onClick={() => handleClick(item.page)}
          aria-label={item.label}
        >
          <span className={styles.icon}>
            <Icon icon={item.icon} size="md" tone="muted" />
          </span>
          <Text size="1" tone="muted" className={styles.label}>
            {item.label}
          </Text>
        </button>
      ))}
    </nav>
  );
};
