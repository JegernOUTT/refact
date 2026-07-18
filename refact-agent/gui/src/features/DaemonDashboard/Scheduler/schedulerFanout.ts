import type {
  CreateCronRequest,
  CronTask,
} from "../../../services/refact/schedulerApi";
import {
  projectApiUrl,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { isReadyWorker } from "../Projects/projectRagStatus";

const MAX_CONCURRENT_REQUESTS = 3;
const LIST_REQUEST_TIMEOUT_MS = 3_000;

export type ProjectCronGroup = {
  projectId: string;
  slug: string;
  tasks: CronTask[];
  error: boolean;
};

export type SchedulerFanoutResult = {
  groups: ProjectCronGroup[];
  hadErrors: boolean;
};

export function formatRelativeMs(timestampMs: number): string {
  const remaining = timestampMs - Date.now();
  if (remaining <= 0) return "due";
  const seconds = Math.round(remaining / 1_000);
  if (seconds < 90) return `in ${String(seconds)}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 90) return `in ${String(minutes)}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `in ${String(hours)}h ${String(minutes % 60)}m`;
  return `in ${String(Math.floor(hours / 24))}d`;
}

async function extractErrorMessage(response: Response): Promise<string> {
  try {
    const data: unknown = await response.json();
    if (data && typeof data === "object") {
      if ("detail" in data && typeof data.detail === "string") {
        return data.detail;
      }
      if ("error" in data && typeof data.error === "string") {
        return data.error;
      }
    }
  } catch {
    return `Scheduler request failed (${String(response.status)})`;
  }
  return `Scheduler request failed (${String(response.status)})`;
}

async function requestJson(
  url: string,
  init: RequestInit = {},
): Promise<unknown> {
  const response = await fetch(url, {
    credentials: "same-origin",
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...init.headers,
    },
  });
  if (!response.ok) {
    throw new Error(await extractErrorMessage(response));
  }
  return (await response.json()) as unknown;
}

export async function fetchProjectCronTasks(
  daemonBase: string,
  projectId: string,
  signal?: AbortSignal,
): Promise<CronTask[]> {
  const controller = new AbortController();
  const timeout = window.setTimeout(
    () => controller.abort(),
    LIST_REQUEST_TIMEOUT_MS,
  );
  const abort = () => controller.abort();
  signal?.addEventListener("abort", abort, { once: true });
  try {
    const response = await fetch(
      projectApiUrl(daemonBase, projectId, "/scheduler/cron"),
      { credentials: "same-origin", signal: controller.signal },
    );
    if (!response.ok) throw new Error("Request failed");
    const data: unknown = await response.json();
    return Array.isArray(data) ? (data as CronTask[]) : [];
  } finally {
    signal?.removeEventListener("abort", abort);
    window.clearTimeout(timeout);
  }
}

export async function fetchCrossProjectCron(
  daemonBase: string,
  workers: DaemonWorker[],
  signal?: AbortSignal,
): Promise<SchedulerFanoutResult> {
  const readyWorkers = workers.filter(isReadyWorker);
  const groups: ProjectCronGroup[] = [];
  let hadErrors = false;
  let nextIndex = 0;

  async function runWorker() {
    while (nextIndex < readyWorkers.length && !signal?.aborted) {
      const worker = readyWorkers[nextIndex];
      nextIndex += 1;
      try {
        const tasks = await fetchProjectCronTasks(
          daemonBase,
          worker.project_id,
          signal,
        );
        groups.push({
          projectId: worker.project_id,
          slug: worker.slug,
          tasks,
          error: false,
        });
      } catch {
        if (signal?.aborted) continue;
        hadErrors = true;
        groups.push({
          projectId: worker.project_id,
          slug: worker.slug,
          tasks: [],
          error: true,
        });
      }
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(MAX_CONCURRENT_REQUESTS, readyWorkers.length) },
      runWorker,
    ),
  );

  groups.sort((left, right) => left.slug.localeCompare(right.slug));
  return { groups, hadErrors };
}

export async function createProjectCron(
  daemonBase: string,
  projectId: string,
  request: CreateCronRequest,
): Promise<void> {
  await requestJson(projectApiUrl(daemonBase, projectId, "/scheduler/cron"), {
    method: "POST",
    body: JSON.stringify(request),
  });
}

export async function setProjectCronEnabled(
  daemonBase: string,
  projectId: string,
  id: string,
  enabled: boolean,
): Promise<void> {
  await requestJson(
    projectApiUrl(
      daemonBase,
      projectId,
      `/scheduler/cron/${encodeURIComponent(id)}`,
    ),
    { method: "PATCH", body: JSON.stringify({ enabled }) },
  );
}

export async function runProjectCron(
  daemonBase: string,
  projectId: string,
  id: string,
): Promise<void> {
  await requestJson(
    projectApiUrl(
      daemonBase,
      projectId,
      `/scheduler/cron/${encodeURIComponent(id)}/run`,
    ),
    { method: "POST" },
  );
}

export async function deleteProjectCron(
  daemonBase: string,
  projectId: string,
  id: string,
): Promise<void> {
  await requestJson(
    projectApiUrl(
      daemonBase,
      projectId,
      `/scheduler/cron/${encodeURIComponent(id)}`,
    ),
    { method: "DELETE" },
  );
}
