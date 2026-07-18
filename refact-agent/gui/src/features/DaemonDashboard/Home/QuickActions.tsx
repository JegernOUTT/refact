import {
  Activity,
  CalendarClock,
  ExternalLink,
  FolderPlus,
  ListRestart,
} from "lucide-react";

import { Button, Surface } from "../../../components/ui";
import type { DashboardPage } from "../dashboardSlice";
import styles from "./Home.module.css";

type QuickActionsProps = {
  setupAvailable: boolean;
  onAddProject: () => void;
  onNavigate: (page: DashboardPage) => void;
  onSetup: () => void;
};

export function QuickActions({
  setupAvailable,
  onAddProject,
  onNavigate,
  onSetup,
}: QuickActionsProps) {
  return (
    <Surface
      as="section"
      className={styles.quickActions}
      radius="card"
      variant="glass"
      aria-labelledby="quick-actions-heading"
    >
      <div className={styles.widgetHeader}>
        <div>
          <h3 id="quick-actions-heading">Quick actions</h3>
          <p>Jump straight to common dashboard work.</p>
        </div>
      </div>
      <div className={styles.actionGrid}>
        <Button leftIcon={FolderPlus} onClick={onAddProject} variant="soft">
          Add project
        </Button>
        <Button
          leftIcon={CalendarClock}
          onClick={() => onNavigate("scheduler")}
          variant="soft"
        >
          Open scheduler
        </Button>
        <Button
          leftIcon={Activity}
          onClick={() => onNavigate("activity")}
          variant="soft"
        >
          Open activity
        </Button>
        <Button asChild leftIcon={ExternalLink} variant="soft">
          <a
            href="https://github.com/JegernOUTT/refact/wiki"
            rel="noreferrer"
            target="_blank"
          >
            Open docs
          </a>
        </Button>
        {setupAvailable ? (
          <Button leftIcon={ListRestart} onClick={onSetup} variant="soft">
            Setup
          </Button>
        ) : null}
      </div>
    </Surface>
  );
}
