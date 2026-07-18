import { useEffect, useMemo, useState } from "react";

import { selectConfig } from "../../Config/configSlice";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  resolveDaemonBaseUrl,
  useCheckDaemonUpdateQuery,
  useListProjectsQuery,
  type DaemonProjectOpenResponse,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { navigateDashboard, type DashboardPage } from "../dashboardSlice";
import { AddProjectDialog } from "../Projects/AddProjectDialog";
import { workerStateName } from "../Projects/projectRagStatus";
import { ContinueWidget } from "./ContinueWidget";
import { FirstRunWizard } from "./FirstRunWizard";
import { fetchHomeFanout, type HomeFanoutResult } from "./homeFanout";
import { NeedsAttentionWidget } from "./NeedsAttentionWidget";
import { QuickActions } from "./QuickActions";
import styles from "./Home.module.css";

export const WIZARD_DONE_KEY = "dashboard:v1:wizard_done";
const WORKERS_POLLING_INTERVAL_MS = 4_000;

const emptyFanout: HomeFanoutResult = {
  chats: [],
  failedCrons: [],
  hadErrors: false,
};

function pendingWorker(response: DaemonProjectOpenResponse): DaemonWorker {
  return {
    project_id: response.project_id,
    slug: response.slug,
    root: response.root,
    pinned: response.pinned,
    last_active_ms: null,
    state: "starting",
    pid: null,
    http_port: null,
    lsp_port: null,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: null,
    last_error: null,
  };
}

function readWizardDone(): boolean {
  try {
    return window.localStorage.getItem(WIZARD_DONE_KEY) === "true";
  } catch {
    return false;
  }
}

export function HomePage() {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const daemonBase = resolveDaemonBaseUrl(config);
  const [wizardDone, setWizardDone] = useState(readWizardDone);
  const [addProjectOpen, setAddProjectOpen] = useState(false);
  const [optimisticWorker, setOptimisticWorker] = useState<DaemonWorker | null>(
    null,
  );
  const [fanout, setFanout] = useState<HomeFanoutResult>(emptyFanout);
  const [fanoutLoading, setFanoutLoading] = useState(false);
  const {
    data: workersData = [],
    fulfilledTimeStamp,
    isLoading,
    refetch,
  } = useListProjectsQuery(undefined, {
    pollingInterval: WORKERS_POLLING_INTERVAL_MS,
  });
  const { data: updateCheck, isLoading: updateLoading } =
    useCheckDaemonUpdateQuery(undefined);

  useEffect(() => {
    if (
      optimisticWorker &&
      workersData.some(
        (worker) =>
          worker.project_id === optimisticWorker.project_id ||
          worker.root === optimisticWorker.root,
      )
    ) {
      setOptimisticWorker(null);
    }
  }, [optimisticWorker, workersData]);

  const workers = useMemo(() => {
    if (
      !optimisticWorker ||
      workersData.some(
        (worker) =>
          worker.project_id === optimisticWorker.project_id ||
          worker.root === optimisticWorker.root,
      )
    ) {
      return workersData;
    }
    return [optimisticWorker, ...workersData];
  }, [optimisticWorker, workersData]);

  useEffect(() => {
    if (!wizardDone) return;
    const controller = new AbortController();
    setFanoutLoading(true);
    void fetchHomeFanout(daemonBase, workers, controller.signal)
      .then((result) => {
        if (!controller.signal.aborted) setFanout(result);
      })
      .finally(() => {
        if (!controller.signal.aborted) setFanoutLoading(false);
      });
    return () => controller.abort();
  }, [daemonBase, fulfilledTimeStamp, wizardDone, workers]);

  function persistWizardDone(done: boolean) {
    setWizardDone(done);
    try {
      if (done) window.localStorage.setItem(WIZARD_DONE_KEY, "true");
      else window.localStorage.removeItem(WIZARD_DONE_KEY);
    } catch {
      return;
    }
  }

  function navigate(page: DashboardPage) {
    dispatch(navigateDashboard({ page, params: {} }));
  }

  function handleProjectOpened(response: DaemonProjectOpenResponse) {
    setOptimisticWorker(pendingWorker(response));
    void refetch();
  }

  const crashedWorkers = workers.filter((worker) => {
    const state = workerStateName(worker);
    return state === "crashed" || state === "failed";
  });

  return (
    <section className={styles.page} aria-labelledby="home-heading">
      <header className={styles.pageHeader}>
        <div>
          <h2 id="home-heading">Home</h2>
          <p>Your projects, recent work, and the next useful move.</p>
        </div>
      </header>

      {!isLoading && !wizardDone ? (
        <FirstRunWizard
          daemonBase={daemonBase}
          onDone={() => persistWizardDone(true)}
          onProjectOpened={handleProjectOpened}
          workers={workers}
        />
      ) : (
        <div className={styles.widgets}>
          <ContinueWidget
            chats={fanout.chats}
            hadErrors={fanout.hadErrors}
            loading={fanoutLoading}
          />
          <NeedsAttentionWidget
            crashedWorkers={crashedWorkers}
            failedCrons={fanout.failedCrons}
            loading={fanoutLoading || updateLoading}
            onNavigate={navigate}
            updateAvailable={updateCheck?.update_available === true}
          />
          <QuickActions
            onAddProject={() => setAddProjectOpen(true)}
            onNavigate={navigate}
            onSetup={() => persistWizardDone(false)}
            setupAvailable={wizardDone}
          />
        </div>
      )}

      <AddProjectDialog
        onFailed={() => setOptimisticWorker(null)}
        onOpenChange={setAddProjectOpen}
        onOpening={() => undefined}
        onProjectOpened={handleProjectOpened}
        open={addProjectOpen}
      />
    </section>
  );
}
