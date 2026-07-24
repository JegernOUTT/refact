import type { ProviderListResponse } from "../../../services/refact/providers";
import type { CronTask } from "../../../services/refact/schedulerApi";
import type { TrajectoryMeta } from "../../../services/refact/trajectories";
import {
  projectApiUrl,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { isReadyWorker } from "../Projects/projectRagStatus";

const MAX_CONCURRENT_REQUESTS = 3;
const REQUEST_TIMEOUT_MS = 3_000;
const MAX_RECENT_PROJECTS = 5;
const MAX_PROJECT_CHATS = 3;

export type RecentProjectChat = {
  id: string;
  projectId: string;
  projectSlug: string;
  title: string;
  updatedAt: string;
};

export type FailedProjectCron = {
  id: string;
  projectId: string;
  projectSlug: string;
  description: string;
  error: string | null;
};

export type HomeFanoutResult = {
  chats: RecentProjectChat[];
  failedCrons: FailedProjectCron[];
  hadErrors: boolean;
};

type FanoutTask =
  | { kind: "trajectories"; worker: DaemonWorker }
  | { kind: "cron"; worker: DaemonWorker };

export function homeFanoutWorkerSignature(workers: DaemonWorker[]): string {
  return workers
    .map((worker) =>
      JSON.stringify([worker.project_id, worker.root, worker.state]),
    )
    .sort()
    .join("|");
}

async function fetchJson(url: string, signal?: AbortSignal): Promise<unknown> {
  const controller = new AbortController();
  const timeout = window.setTimeout(
    () => controller.abort(),
    REQUEST_TIMEOUT_MS,
  );
  const abort = () => controller.abort();
  signal?.addEventListener("abort", abort, { once: true });
  try {
    const response = await fetch(url, {
      credentials: "same-origin",
      signal: controller.signal,
    });
    if (!response.ok) throw new Error("Request failed");
    return (await response.json()) as unknown;
  } finally {
    signal?.removeEventListener("abort", abort);
    window.clearTimeout(timeout);
  }
}

function trajectoryItems(data: unknown): TrajectoryMeta[] {
  if (Array.isArray(data)) return data as TrajectoryMeta[];
  if (
    data &&
    typeof data === "object" &&
    "items" in data &&
    Array.isArray(data.items)
  ) {
    return data.items as TrajectoryMeta[];
  }
  return [];
}

function cronItems(data: unknown): CronTask[] {
  return Array.isArray(data) ? (data as CronTask[]) : [];
}

function cronFailed(task: CronTask): boolean {
  const statuses = [
    task.last_status,
    ...task.recent_runs.map((run) => run.status),
  ]
    .filter((status): status is string => typeof status === "string")
    .map((status) => status.toLowerCase());
  return (
    Boolean(task.last_error) ||
    task.recent_runs.some((run) => Boolean(run.error)) ||
    statuses.some((status) => status === "failed" || status === "error")
  );
}

export async function probeProjectProviders(
  daemonBase: string,
  projectId: string,
  signal?: AbortSignal,
): Promise<boolean> {
  const data = await fetchJson(
    projectApiUrl(daemonBase, projectId, "/providers"),
    signal,
  );
  const providers =
    data &&
    typeof data === "object" &&
    "providers" in data &&
    Array.isArray(data.providers)
      ? (data as ProviderListResponse).providers
      : [];
  return providers.some(
    (provider) =>
      provider.status === "configured" || provider.status === "active",
  );
}

export async function fetchHomeFanout(
  daemonBase: string,
  workers: DaemonWorker[],
  signal?: AbortSignal,
): Promise<HomeFanoutResult> {
  const readyWorkers = workers.filter(isReadyWorker);
  const recentWorkers = [...readyWorkers]
    .sort(
      (left, right) => (right.last_active_ms ?? 0) - (left.last_active_ms ?? 0),
    )
    .slice(0, MAX_RECENT_PROJECTS);
  const tasks: FanoutTask[] = [
    ...recentWorkers.map(
      (worker): FanoutTask => ({ kind: "trajectories", worker }),
    ),
    ...readyWorkers.map((worker): FanoutTask => ({ kind: "cron", worker })),
  ];
  const chats: RecentProjectChat[] = [];
  const failedCrons: FailedProjectCron[] = [];
  let hadErrors = false;
  let nextIndex = 0;

  async function runWorker() {
    while (nextIndex < tasks.length && !signal?.aborted) {
      const task = tasks[nextIndex];
      nextIndex += 1;
      try {
        if (task.kind === "trajectories") {
          const data = await fetchJson(
            projectApiUrl(
              daemonBase,
              task.worker.project_id,
              `/trajectories?limit=${String(
                MAX_PROJECT_CHATS,
              )}&displayable_only=true`,
            ),
            signal,
          );
          chats.push(
            ...trajectoryItems(data)
              .sort(
                (left, right) =>
                  Date.parse(right.updated_at) - Date.parse(left.updated_at),
              )
              .slice(0, MAX_PROJECT_CHATS)
              .map((trajectory) => ({
                id: trajectory.id,
                projectId: task.worker.project_id,
                projectSlug: task.worker.slug,
                title: trajectory.title || "Untitled chat",
                updatedAt: trajectory.updated_at,
              })),
          );
        } else {
          const data = await fetchJson(
            projectApiUrl(
              daemonBase,
              task.worker.project_id,
              "/scheduler/cron",
            ),
            signal,
          );
          failedCrons.push(
            ...cronItems(data)
              .filter(cronFailed)
              .map((cron) => ({
                id: cron.id,
                projectId: task.worker.project_id,
                projectSlug: task.worker.slug,
                description: cron.description || "Scheduled task",
                error:
                  cron.last_error ??
                  cron.recent_runs.find((run) => run.error)?.error ??
                  null,
              })),
          );
        }
      } catch {
        if (!signal?.aborted) hadErrors = true;
      }
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(MAX_CONCURRENT_REQUESTS, tasks.length) },
      runWorker,
    ),
  );

  chats.sort(
    (left, right) => Date.parse(right.updatedAt) - Date.parse(left.updatedAt),
  );
  return { chats, failedCrons, hadErrors };
}
