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
  established: boolean;
};

export type WizardEvent =
  | { type: "add_project" }
  | { type: "project_opening" }
  | { type: "project_opened"; projectId: string }
  | { type: "workers_updated"; workers: DaemonWorker[] }
  | { type: "providers_checked"; configured: boolean }
  | { type: "providers_check_failed" }
  | { type: "recheck_providers" }
  | { type: "chats_detected" }
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
  const ranked = [...readyWorkers].sort((left, right) => {
    if (left.pinned !== right.pinned) return left.pinned ? -1 : 1;
    return (right.last_active_ms ?? 0) - (left.last_active_ms ?? 0);
  });
  return ranked[0];
}

export function createWizardState(
  workers: DaemonWorker[],
  dismissed: boolean,
  userRequested = false,
): WizardState {
  const established = !userRequested && workers.length > 0;
  if (dismissed) {
    return {
      step: "done",
      projectId: null,
      providerProbeFailed: false,
      established,
    };
  }
  if (workers.length === 0) {
    return {
      step: "no_projects",
      projectId: null,
      providerProbeFailed: false,
      established,
    };
  }
  const readyWorker = preferredReadyWorker(workers, null);
  if (readyWorker) {
    return {
      step: "provider_check",
      projectId: readyWorker.project_id,
      providerProbeFailed: false,
      established,
    };
  }
  return {
    step: "project_starting",
    projectId: workers[0]?.project_id ?? null,
    providerProbeFailed: false,
    established,
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
        ...state,
        step: "project_starting",
        providerProbeFailed: false,
      };
    case "project_opened":
      return {
        ...state,
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
              ...state,
              step: "no_projects",
              projectId: null,
              providerProbeFailed: false,
            };
      }
      const readyWorker = preferredReadyWorker(event.workers, state.projectId);
      if (!readyWorker) {
        return {
          ...state,
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
        ...state,
        step: "provider_check",
        projectId: readyWorker.project_id,
        providerProbeFailed: false,
      };
    }
    case "providers_checked": {
      if (state.step !== "provider_check") return state;
      if (event.configured && state.established) {
        return { ...state, step: "done", providerProbeFailed: false };
      }
      return {
        ...state,
        step: event.configured ? "ready_for_chat" : "provider_setup_pointer",
        providerProbeFailed: false,
      };
    }
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
    case "chats_detected":
      return state.established && state.step !== "done"
        ? { ...state, step: "done", providerProbeFailed: false }
        : state;
    case "skip":
    case "complete":
      return {
        ...state,
        step: "done",
        providerProbeFailed: false,
      };
    case "restart":
      return createWizardState(event.workers, false, true);
  }
}
