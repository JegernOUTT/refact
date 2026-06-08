import React, { useCallback } from "react";
import classNames from "classnames";
import { BarChart3, Box, Settings } from "lucide-react";
import { Icon } from "../../../../components/ui";
import { DashboardText as Text } from "../DashboardPrimitives";
import { useAppDispatch, useAppSelector } from "../../../../hooks";
import { push, selectCurrentPage, type Page } from "../../../Pages/pagesSlice";
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
    icon: Settings,
    label: "Settings",
    page: { name: "general settings" },
  },
  {
    icon: Box,
    label: "Marketplace",
    page: { name: "marketplace hub" },
  },
];

function isActivePage(currentPage: Page | undefined, itemPage: Page): boolean {
  return currentPage?.name === itemPage.name;
}

export const NavBar: React.FC = () => {
  const dispatch = useAppDispatch();
  const currentPage = useAppSelector(selectCurrentPage);

  const handleClick = useCallback(
    (page: Page) => {
      dispatch(push(page));
    },
    [dispatch],
  );

  return (
    <nav className={`${styles.nav} rf-glass-panel rf-enter-rise rf-stagger`}>
      {NAV_ITEMS.map((item) => {
        const active = isActivePage(currentPage, item.page);

        return (
          <button
            key={item.label}
            type="button"
            className={classNames(styles.navButton, "rf-pressable")}
            onClick={() => handleClick(item.page)}
            aria-label={item.label}
            aria-current={active ? "page" : undefined}
            data-active={active || undefined}
          >
            <span className={styles.icon}>
              <Icon icon={item.icon} size="md" tone="muted" />
            </span>
            <Text size="1" tone="muted" className={styles.label}>
              {item.label}
            </Text>
          </button>
        );
      })}
    </nav>
  );
};
