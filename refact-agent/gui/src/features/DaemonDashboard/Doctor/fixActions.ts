import type { ProviderDefaults } from "../../../services/refact/providers";
import { projectApiUrl } from "../../../services/refact/daemon";
import type { DoctorFix, StaleModelFix } from "./clientChecks";

export const PRUNE_CACHES_COMMAND = "du -sh ~/.cache/refact/*";
export const PRUNE_CACHES_LABEL = "Inspect cache usage";
export const PRUNE_CACHES_HINT =
  "Nothing is deleted automatically. After inspecting, it is safe to manually remove worktrees of merged branches under ~/.cache/refact/worktrees and old logs under ~/.cache/refact/logs.";

const RESTART_WORKER_PREFIX = "restart_worker:";

export function resolveServerFixAction(
  fixAction: string | null | undefined,
): DoctorFix | null {
  if (!fixAction) return null;
  if (fixAction.startsWith(RESTART_WORKER_PREFIX)) {
    const projectId = fixAction.slice(RESTART_WORKER_PREFIX.length);
    return projectId ? { kind: "restart_worker", projectId } : null;
  }
  switch (fixAction) {
    case "run_update":
      return { kind: "run_update" };
    case "open_settings":
      return { kind: "open_settings" };
    case "prune_caches":
      return {
        kind: "copy_command",
        command: PRUNE_CACHES_COMMAND,
        label: PRUNE_CACHES_LABEL,
        hint: PRUNE_CACHES_HINT,
      };
    default:
      return null;
  }
}

export function buildDefaultsUpdate(
  fix: StaleModelFix,
  model: string,
): ProviderDefaults {
  const slot = fix.defaults[fix.slotKey] ?? {};
  return { ...fix.defaults, [fix.slotKey]: { ...slot, model } };
}

export async function applyStaleModelFix(
  daemonBase: string,
  fix: StaleModelFix,
  model: string,
): Promise<void> {
  const response = await fetch(
    projectApiUrl(daemonBase, fix.projectId, "/defaults"),
    {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(buildDefaultsUpdate(fix, model)),
    },
  );
  if (!response.ok) throw new Error("Failed to update default models");
}
