import { useState } from "react";
import { ArrowLeft, ExternalLink, FolderKanban } from "lucide-react";

import {
  Badge,
  Button,
  EmptyState,
  LoadingState,
  Tabs,
} from "../../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../../hooks";
import { selectConfig } from "../../../Config/configSlice";
import {
  resolveDaemonBaseUrl,
  resolveDaemonLogsUrl,
  useListProjectsQuery,
} from "../../../../services/refact/daemon";
import { navigateDashboard } from "../../dashboardSlice";
import { workerPresentation } from "../projectRagStatus";
import { ActivityTab } from "./tabs/ActivityTab";
import { ChatsTab } from "./tabs/ChatsTab";
import { GitTab } from "./tabs/GitTab";
import { HealthTab } from "./tabs/HealthTab";
import { OverviewTab } from "./tabs/OverviewTab";
import { SettingsTab } from "./tabs/SettingsTab";
import { TasksTab } from "./tabs/TasksTab";
import styles from "./ProjectDetail.module.css";

const WORKERS_POLLING_INTERVAL_MS = 4_000;
const LOG_TAIL_LINES = 500;

const TAB_VALUES = [
  "overview",
  "health",
  "git",
  "activity",
  "chats",
  "tasks",
  "settings",
] as const;

type ProjectDetailPageProps = {
  projectId: string;
};

export function ProjectDetailPage({ projectId }: ProjectDetailPageProps) {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const daemonBase = resolveDaemonBaseUrl(config);
  const [tab, setTab] = useState<(typeof TAB_VALUES)[number]>("overview");
  const { data, isLoading, refetch } = useListProjectsQuery(undefined, {
    pollingInterval: WORKERS_POLLING_INTERVAL_MS,
  });
  const worker = data?.find((row) => row.project_id === projectId);
  const backButton = (
    <Button
      leftIcon={ArrowLeft}
      onClick={() =>
        dispatch(navigateDashboard({ page: "projects", params: {} }))
      }
      size="sm"
      variant="ghost"
    >
      All projects
    </Button>
  );

  if (!worker) {
    if (isLoading) {
      return <LoadingState label="Loading project" variant="full" />;
    }
    return (
      <EmptyState
        action={backButton}
        description="This project is no longer registered with the daemon."
        icon={FolderKanban}
        title="Project not found"
        variant="full"
      />
    );
  }

  const presentation = workerPresentation(worker);
  const openUrl = `${daemonBase.replace(/\/+$/, "")}/p/${encodeURIComponent(
    worker.project_id,
  )}/`;
  const logsUrl = resolveDaemonLogsUrl(
    config,
    worker.project_id,
    false,
    LOG_TAIL_LINES,
  );
  const onMutated = () => void refetch();

  return (
    <section aria-labelledby="project-detail-heading" className={styles.page}>
      <header className={styles.pageHeader}>
        <div className={styles.identity}>
          {backButton}
          <h2 className={styles.title} id="project-detail-heading">
            {worker.slug}
          </h2>
          <span className={styles.root} title={worker.root}>
            {worker.root}
          </span>
        </div>
        <div className={styles.headerMeta}>
          <Badge tone={presentation.tone} variant="soft">
            {presentation.label}
          </Badge>
          {worker.pinned ? <Badge variant="soft">Pinned</Badge> : null}
          <Button asChild leftIcon={ExternalLink} size="sm" variant="primary">
            <a href={openUrl}>Open workspace</a>
          </Button>
        </div>
      </header>

      <Tabs
        onValueChange={(value) => setTab(value as (typeof TAB_VALUES)[number])}
        value={tab}
      >
        <Tabs.List
          activeIndex={TAB_VALUES.indexOf(tab)}
          itemCount={TAB_VALUES.length}
        >
          <Tabs.Trigger value="overview">Overview</Tabs.Trigger>
          <Tabs.Trigger value="health">Health</Tabs.Trigger>
          <Tabs.Trigger value="git">Git</Tabs.Trigger>
          <Tabs.Trigger value="activity">Activity</Tabs.Trigger>
          <Tabs.Trigger value="chats">Chats</Tabs.Trigger>
          <Tabs.Trigger value="tasks">Tasks</Tabs.Trigger>
          <Tabs.Trigger value="settings">Settings</Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="overview">
          <OverviewTab
            daemonBase={daemonBase}
            onMutated={onMutated}
            worker={worker}
          />
        </Tabs.Content>
        <Tabs.Content value="health">
          <HealthTab
            daemonBase={daemonBase}
            onMutated={onMutated}
            openUrl={openUrl}
            worker={worker}
          />
        </Tabs.Content>
        <Tabs.Content value="git">
          <GitTab
            daemonBase={daemonBase}
            onMutated={onMutated}
            worker={worker}
          />
        </Tabs.Content>
        <Tabs.Content value="activity">
          <ActivityTab worker={worker} />
        </Tabs.Content>
        <Tabs.Content value="chats">
          <ChatsTab
            daemonBase={daemonBase}
            onMutated={onMutated}
            openUrl={openUrl}
            worker={worker}
          />
        </Tabs.Content>
        <Tabs.Content value="tasks">
          <TasksTab
            daemonBase={daemonBase}
            onMutated={onMutated}
            openUrl={openUrl}
            worker={worker}
          />
        </Tabs.Content>
        <Tabs.Content value="settings">
          <SettingsTab
            logsUrl={logsUrl}
            onMutated={onMutated}
            worker={worker}
          />
        </Tabs.Content>
      </Tabs>
    </section>
  );
}
