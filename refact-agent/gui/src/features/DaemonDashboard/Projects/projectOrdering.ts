import type { DaemonWorker } from "../../../services/refact/daemon";
import { workerStateName } from "./projectRagStatus";

const STATE_RANK = new Map<string, number>([
  ["ready", 1],
  ["starting", 2],
  ["stopping", 3],
  ["crashed", 4],
  ["failed", 4],
  ["stopped", 5],
]);

const UNKNOWN_STATE_RANK = 6;
const PINNED_RANK = 0;

export function isMissingWorker(worker: DaemonWorker): boolean {
  return worker.root_exists === false;
}

function workerRank(worker: DaemonWorker): number {
  if (worker.pinned) return PINNED_RANK;
  return STATE_RANK.get(workerStateName(worker)) ?? UNKNOWN_STATE_RANK;
}

function compareAlphabetical(a: DaemonWorker, b: DaemonWorker): number {
  return (
    a.slug.localeCompare(b.slug, undefined, { sensitivity: "base" }) ||
    a.project_id.localeCompare(b.project_id)
  );
}

export type SplitProjectWorkers = {
  present: DaemonWorker[];
  missing: DaemonWorker[];
};

export function splitProjectWorkers(
  workers: DaemonWorker[],
): SplitProjectWorkers {
  const present = workers.filter((worker) => !isMissingWorker(worker));
  const missing = workers.filter(isMissingWorker);
  present.sort(
    (a, b) => workerRank(a) - workerRank(b) || compareAlphabetical(a, b),
  );
  missing.sort(compareAlphabetical);
  return { present, missing };
}
