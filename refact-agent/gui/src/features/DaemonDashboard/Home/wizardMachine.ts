import type { DaemonWorker } from "../../../services/refact/daemon";
import { isReadyWorker } from "../Projects/projectRagStatus";

export type WizardStep =
  | "no_projects"
  | "adding_project"
  | "project_starting"
  | "provider_check"
  | "provider_setup_pointer"
  | "ready_for_chat"
  | "done";

export type WizardState = {
  step: WizardStep;
  projectId: string | null;
  providerProbeFailed: boolean;
};

export type WizardEvent =
  | { type: "add_project" }
  | { type: "project_opening" }
  | { type: "project_opened"; projectId: string }
  | { type: "workers_updated"; workers: DaemonWorker[] }
  | { type: "providers_checked"; configured: boolean }
  | { type: "providers_check_failed" }
  | { type: "recheck_providers" }
  | { type: "skip" }
  | { type: "complete" }
  | { type: "restart"; workers: DaemonWorker[] };

function preferredReadyWorker(
  workers: DaemonWorker[],
  projectId: string | null,
): DaemonWorker | null {
  const readyWorkers = workers.filter(isReadyWorker);
  const preferred = readyWorkers.find(
    (worker) => worker.project_id === projectId,
  );
  if (preferred) return preferred;
  if (readyWorkers.length === 0) return null;
  readyWorkers.sort(
    (left, right) => (right.last_active_ms ?? 0) - (left.last_active_ms ?? 0),
  );
  return readyWorkers[0];
}

export function createWizardState(
  workers: DaemonWorker[],
  dismissed: boolean,
): WizardState {
  if (dismissed) {
    return { step: "done", projectId: null, providerProbeFailed: false };
  }
  if (workers.length === 0) {
    return {
      step: "no_projects",
      projectId: null,
      providerProbeFailed: false,
    };
  }
  const readyWorker = preferredReadyWorker(workers, null);
  if (readyWorker) {
    return {
      step: "provider_check",
      projectId: readyWorker.project_id,
      providerProbeFailed: false,
    };
  }
  return {
    step: "project_starting",
    projectId: workers[0]?.project_id ?? null,
    providerProbeFailed: false,
  };
}

export function wizardReducer(
  state: WizardState,
  event: WizardEvent,
): WizardState {
  switch (event.type) {
    case "add_project":
      return state.step === "no_projects"
        ? { ...state, step: "adding_project" }
        : state;
    case "project_opening":
      return {
        step: "project_starting",
        projectId: state.projectId,
        providerProbeFailed: false,
      };
    case "project_opened":
      return {
        step: "project_starting",
        projectId: event.projectId,
        providerProbeFailed: false,
      };
    case "workers_updated": {
      if (state.step === "done") return state;
      if (event.workers.length === 0) {
        return state.step === "adding_project"
          ? state
          : {
              step: "no_projects",
              projectId: null,
              providerProbeFailed: false,
            };
      }
      const readyWorker = preferredReadyWorker(event.workers, state.projectId);
      if (!readyWorker) {
        return {
          step: "project_starting",
          projectId: state.projectId ?? event.workers[0].project_id,
          providerProbeFailed: false,
        };
      }
      if (
        state.projectId === readyWorker.project_id &&
        (state.step === "provider_setup_pointer" ||
          state.step === "ready_for_chat" ||
          state.step === "provider_check")
      ) {
        return state;
      }
      return {
        step: "provider_check",
        projectId: readyWorker.project_id,
        providerProbeFailed: false,
      };
    }
    case "providers_checked":
      return state.step === "provider_check"
        ? {
            ...state,
            step: event.configured
              ? "ready_for_chat"
              : "provider_setup_pointer",
            providerProbeFailed: false,
          }
        : state;
    case "providers_check_failed":
      return state.step === "provider_check"
        ? {
            ...state,
            step: "provider_setup_pointer",
            providerProbeFailed: true,
          }
        : state;
    case "recheck_providers":
      return state.step === "provider_setup_pointer"
        ? {
            ...state,
            step: "provider_check",
            providerProbeFailed: false,
          }
        : state;
    case "skip":
    case "complete":
      return {
        step: "done",
        projectId: state.projectId,
        providerProbeFailed: false,
      };
    case "restart":
      return createWizardState(event.workers, false);
  }
}
