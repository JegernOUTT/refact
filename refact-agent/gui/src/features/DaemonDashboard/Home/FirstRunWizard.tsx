import { useEffect, useReducer, useState } from "react";
import {
  CheckCircle2,
  ExternalLink,
  FolderPlus,
  KeyRound,
  MessageSquarePlus,
  X,
} from "lucide-react";

import {
  Button,
  Icon,
  IconButton,
  LoadingState,
  Surface,
} from "../../../components/ui";
import type {
  DaemonProjectOpenResponse,
  DaemonWorker,
} from "../../../services/refact/daemon";
import { AddProjectDialog } from "../Projects/AddProjectDialog";
import { probeProjectProviders } from "./homeFanout";
import {
  createWizardState,
  wizardReducer,
  type WizardStep,
} from "./wizardMachine";
import styles from "./Home.module.css";

type FirstRunWizardProps = {
  daemonBase: string;
  workers: DaemonWorker[];
  hasChats: boolean;
  userRequested: boolean;
  onDone: () => void;
  onProjectOpened: (response: DaemonProjectOpenResponse) => void;
};

const stepNumber: Partial<Record<WizardStep, number>> = {
  no_projects: 1,
  adding_project: 1,
  project_starting: 1,
  provider_check: 2,
  provider_setup_pointer: 2,
  ready_for_chat: 3,
};

export function FirstRunWizard({
  daemonBase,
  workers,
  hasChats,
  userRequested,
  onDone,
  onProjectOpened,
}: FirstRunWizardProps) {
  const [state, dispatch] = useReducer(
    wizardReducer,
    workers,
    (initialWorkers) => createWizardState(initialWorkers, false, userRequested),
  );
  const [dialogOpen, setDialogOpen] = useState(false);

  useEffect(() => {
    dispatch({ type: "workers_updated", workers });
  }, [workers]);

  useEffect(() => {
    if (hasChats) dispatch({ type: "chats_detected" });
  }, [hasChats]);

  useEffect(() => {
    if (state.step === "done") onDone();
  }, [onDone, state.step]);

  useEffect(() => {
    if (state.step !== "provider_check" || !state.projectId) return;
    const controller = new AbortController();
    void probeProjectProviders(daemonBase, state.projectId, controller.signal)
      .then((configured) => {
        dispatch({ type: "providers_checked", configured });
      })
      .catch(() => {
        if (!controller.signal.aborted) {
          dispatch({ type: "providers_check_failed" });
        }
      });
    return () => controller.abort();
  }, [daemonBase, state.projectId, state.step]);

  function finish() {
    dispatch({ type: "complete" });
    onDone();
  }

  function skip() {
    dispatch({ type: "skip" });
    onDone();
  }

  function openAddProject() {
    dispatch({ type: "add_project" });
    setDialogOpen(true);
  }

  const selectedWorker =
    workers.find((worker) => worker.project_id === state.projectId) ?? null;
  const projectHref = state.projectId
    ? `/p/${encodeURIComponent(state.projectId)}/`
    : "";
  const providersHref = projectHref ? `${projectHref}?page=providers` : "";

  if (state.step === "done") return null;

  return (
    <Surface
      as="section"
      className={styles.wizard}
      radius="card"
      variant="glass"
      aria-labelledby="wizard-heading"
    >
      <div className={styles.wizardHeader}>
        <div>
          <span className={styles.eyebrow}>First-run setup</span>
          <p className={styles.stepLabel}>
            Step {stepNumber[state.step] ?? 1} of 3
          </p>
        </div>
        <IconButton
          aria-label="Dismiss setup"
          icon={X}
          onClick={skip}
          size="sm"
          variant="ghost"
        />
      </div>

      {state.step === "no_projects" || state.step === "adding_project" ? (
        <div className={styles.heroContent}>
          <div className={styles.heroTitle}>
            <Icon icon={FolderPlus} size="lg" tone="accent" />
            <h2 id="wizard-heading">Bring your first project into Refact</h2>
          </div>
          <p>
            Pick a folder and the daemon will start its worker and prepare the
            workspace.
          </p>
          <Button
            leftIcon={FolderPlus}
            onClick={openAddProject}
            size="lg"
            variant="primary"
          >
            Add project
          </Button>
        </div>
      ) : null}

      {state.step === "project_starting" ? (
        <div className={styles.heroContent}>
          <LoadingState label="Starting your project worker" />
          <h2 id="wizard-heading">Waking up your workspace</h2>
          <p>
            {selectedWorker?.slug ?? "Your project"} will continue automatically
            when its worker is ready.
          </p>
        </div>
      ) : null}

      {state.step === "provider_check" ? (
        <div className={styles.heroContent}>
          <LoadingState label="Checking provider setup" />
          <h2 id="wizard-heading">Checking your model connection</h2>
          <p>
            This quick check keeps the first chat from starting with a snag.
          </p>
        </div>
      ) : null}

      {state.step === "provider_setup_pointer" ? (
        <div className={styles.heroContent}>
          <div className={styles.heroTitle}>
            <Icon icon={KeyRound} size="lg" tone="warning" />
            <h2 id="wizard-heading">Set up a provider</h2>
          </div>
          <p>
            {state.providerProbeFailed
              ? "We could not verify a configured provider. Open the workspace to check it, then try again."
              : "Choose a provider and model in the project workspace, then come back for one quick recheck."}
          </p>
          <div className={styles.heroActions}>
            <Button asChild leftIcon={ExternalLink} size="lg" variant="primary">
              <a href={providersHref}>Open provider setup</a>
            </Button>
            <Button
              onClick={() => dispatch({ type: "recheck_providers" })}
              size="lg"
              variant="soft"
            >
              I&apos;ve done this
            </Button>
          </div>
        </div>
      ) : null}

      {state.step === "ready_for_chat" ? (
        <div className={styles.heroContent}>
          <div className={styles.heroTitle}>
            <Icon icon={CheckCircle2} size="lg" tone="success" />
            <h2 id="wizard-heading">Your workspace is ready</h2>
          </div>
          <p>
            Start the first chat in {selectedWorker?.slug ?? "your project"}.
          </p>
          <Button
            asChild
            leftIcon={MessageSquarePlus}
            size="lg"
            variant="primary"
          >
            <a href={projectHref} onClick={finish}>
              Start first chat
            </a>
          </Button>
        </div>
      ) : null}

      <div className={styles.wizardFooter}>
        <button className={styles.skipButton} onClick={skip} type="button">
          Skip setup
        </button>
        <span>You can reopen Setup from Quick actions.</span>
      </div>

      <AddProjectDialog
        onFailed={() => dispatch({ type: "workers_updated", workers })}
        onOpenChange={setDialogOpen}
        onOpening={() => dispatch({ type: "project_opening" })}
        onProjectOpened={(response) => {
          dispatch({ type: "project_opened", projectId: response.project_id });
          onProjectOpened(response);
        }}
        open={dialogOpen}
      />
    </Surface>
  );
}
