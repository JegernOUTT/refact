import {
  projectApiUrl,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { isRagStatus, type RagStatus } from "../../../services/refact/types";

const MAX_CONCURRENT_STATUS_REQUESTS = 3;
const STATUS_REQUEST_TIMEOUT_MS = 3_000;

export type ProjectRagStatus =
  | { state: "loading" }
  | { state: "ready"; data: RagStatus }
  | { state: "error" };

export function workerStateName(worker: DaemonWorker): string {
  const state: unknown = worker.state;
  return typeof state === "string" ? state : "failed";
}

export function isReadyWorker(worker: DaemonWorker): boolean {
  return workerStateName(worker) === "ready";
}

async function fetchProjectStatus(
  daemonBase: string,
  worker: DaemonWorker,
): Promise<ProjectRagStatus> {
  const controller = new AbortController();
  const timeout = window.setTimeout(
    () => controller.abort(),
    STATUS_REQUEST_TIMEOUT_MS,
  );
  try {
    const response = await fetch(
      projectApiUrl(daemonBase, worker.project_id, "/rag-status"),
      {
        credentials: "same-origin",
        signal: controller.signal,
      },
    );
    if (!response.ok) return { state: "error" };
    const data: unknown = await response.json();
    return isRagStatus(data) ? { state: "ready", data } : { state: "error" };
  } catch {
    return { state: "error" };
  } finally {
    window.clearTimeout(timeout);
  }
}

export async function fetchReadyProjectStatuses(
  daemonBase: string,
  workers: DaemonWorker[],
): Promise<Record<string, ProjectRagStatus>> {
  const readyWorkers = workers.filter(isReadyWorker);
  const results: Record<string, ProjectRagStatus> = {};
  let nextIndex = 0;

  async function runWorker() {
    while (nextIndex < readyWorkers.length) {
      const worker = readyWorkers[nextIndex];
      nextIndex += 1;
      results[worker.project_id] = await fetchProjectStatus(daemonBase, worker);
    }
  }

  await Promise.all(
    Array.from(
      {
        length: Math.min(MAX_CONCURRENT_STATUS_REQUESTS, readyWorkers.length),
      },
      runWorker,
    ),
  );
  return results;
}
