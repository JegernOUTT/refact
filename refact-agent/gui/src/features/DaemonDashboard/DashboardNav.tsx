import {
  Activity,
  CalendarClock,
  ChartNoAxesCombined,
  FolderKanban,
  House,
  Settings,
  Stethoscope,
  type LucideIcon,
} from "lucide-react";

import { Icon } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  navigateDashboard,
  selectDashboardPage,
  type DashboardPage,
} from "./dashboardSlice";
import styles from "./DaemonDashboard.module.css";

type DashboardNavItem = {
  page: DashboardPage;
  label: string;
  icon: LucideIcon;
};

const NAV_ITEMS: DashboardNavItem[] = [
  { page: "home", label: "Home", icon: House },
  { page: "projects", label: "Projects", icon: FolderKanban },
  { page: "activity", label: "Activity", icon: Activity },
  { page: "scheduler", label: "Scheduler", icon: CalendarClock },
  { page: "usage", label: "Usage", icon: ChartNoAxesCombined },
  { page: "doctor", label: "Doctor", icon: Stethoscope },
  { page: "settings", label: "Settings", icon: Settings },
];

export function DashboardNav() {
  const dispatch = useAppDispatch();
  const currentPage = useAppSelector(selectDashboardPage);

  return (
    <nav className={styles.nav} aria-label="Dashboard navigation">
      {NAV_ITEMS.map((item) => {
        const active = item.page === currentPage;
        return (
          <button
            key={item.page}
            type="button"
            className={styles.navItem}
            aria-current={active ? "page" : undefined}
            aria-label={item.label}
            title={item.label}
            onClick={() =>
              dispatch(navigateDashboard({ page: item.page, params: {} }))
            }
          >
            <Icon icon={item.icon} tone={active ? "accent" : "muted"} />
            <span className={styles.navLabel}>{item.label}</span>
          </button>
        );
      })}
    </nav>
  );
}
