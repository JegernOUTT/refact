import type {
  OpenRouterAccountInfoResponse,
  OpenRouterHealthResponse,
  ProviderDefaults,
  ProviderListItem,
  ProviderListResponse,
} from "../../../services/refact/providers";
import {
  projectApiUrl,
  type DaemonWorker,
} from "../../../services/refact/daemon";
import { isReadyWorker } from "../Projects/projectRagStatus";
import { resolveServerFixAction } from "./fixActions";

const MAX_CONCURRENT_PROJECTS = 3;
const REQUEST_TIMEOUT_MS = 5_000;
const MAX_MODEL_PROVIDERS = 5;
const MAX_HEALTH_CHECKS_PER_PROJECT = 3;

const HEALTH_CAPABLE_BASE_PROVIDERS = new Set(["openrouter", "google_gemini"]);

export type DoctorSeverity = "info" | "warning" | "critical";

export const DEFAULT_MODEL_SLOTS = [
  { key: "chat", label: "Chat" },
  { key: "chat_model_2", label: "Secondary chat" },
  { key: "task_planner_agent_model", label: "Task planner" },
  { key: "chat_light", label: "Light chat" },
  { key: "chat_thinking", label: "Thinking chat" },
  { key: "chat_buddy", label: "Buddy chat" },
] as const;

export type DefaultModelSlotKey = (typeof DEFAULT_MODEL_SLOTS)[number]["key"];

export type StaleModelFix = {
  kind: "stale_default_model";
  projectId: string;
  projectSlug: string;
  slotKey: DefaultModelSlotKey;
  staleModel: string;
  availableModels: string[];
  defaults: ProviderDefaults;
};

export type DoctorFix =
  | StaleModelFix
  | { kind: "restart_worker"; projectId: string }
  | { kind: "run_update" }
  | { kind: "open_settings" }
  | { kind: "copy_command"; command: string }
  | { kind: "open_project_providers"; projectId: string; projectSlug: string };

export type DoctorFinding = {
  id: string;
  severity: DoctorSeverity;
  message: string;
  detail: string | null;
  fix: DoctorFix | null;
};

export type DoctorProject = {
  projectId: string;
  projectSlug: string;
};

type ServerFinding = {
  id: string;
  severity: string;
  message: string;
  detail?: string | null;
  fix_action?: string | null;
};

async function fetchWithTimeout(
  url: string,
  signal?: AbortSignal,
): Promise<Response> {
  const controller = new AbortController();
  const timeout = window.setTimeout(
    () => controller.abort(),
    REQUEST_TIMEOUT_MS,
  );
  const abort = () => controller.abort();
  signal?.addEventListener("abort", abort, { once: true });
  try {
    return await fetch(url, {
      credentials: "same-origin",
      signal: controller.signal,
    });
  } finally {
    signal?.removeEventListener("abort", abort);
    window.clearTimeout(timeout);
  }
}

async function fetchJson(url: string, signal?: AbortSignal): Promise<unknown> {
  const response = await fetchWithTimeout(url, signal);
  if (!response.ok) throw new Error("Request failed");
  return (await response.json()) as unknown;
}

function providerItems(data: unknown): ProviderListItem[] {
  if (
    data &&
    typeof data === "object" &&
    "providers" in data &&
    Array.isArray(data.providers)
  ) {
    return (data as ProviderListResponse).providers;
  }
  return [];
}

export function collectProviderModels(
  providerName: string,
  data: unknown,
): string[] {
  if (!data || typeof data !== "object") return [];
  const models: string[] = [];
  for (const group of ["chat_models", "completion_models"]) {
    const value = (data as Record<string, unknown>)[group];
    if (!Array.isArray(value)) continue;
    for (const entry of value as unknown[]) {
      if (!entry || typeof entry !== "object") continue;
      const name = (entry as { name?: unknown }).name;
      if (typeof name === "string" && name.length > 0) {
        models.push(`${providerName}/${name}`);
      }
    }
  }
  return models;
}

export function staleDefaultModelFindings(
  project: DoctorProject,
  defaults: ProviderDefaults,
  availableModels: string[],
): DoctorFinding[] {
  if (availableModels.length === 0) return [];
  const available = new Set(availableModels);
  const findings: DoctorFinding[] = [];
  for (const slot of DEFAULT_MODEL_SLOTS) {
    const model = defaults[slot.key]?.model?.trim();
    if (!model || available.has(model)) continue;
    findings.push({
      id: `stale_default_model:${project.projectId}:${slot.key}`,
      severity: "critical",
      message: `Default model '${model}' not found on ${project.projectSlug}`,
      detail: `The ${slot.label} slot points at a model no configured provider lists. Pick a replacement and apply it.`,
      fix: {
        kind: "stale_default_model",
        projectId: project.projectId,
        projectSlug: project.projectSlug,
        slotKey: slot.key,
        staleModel: model,
        availableModels,
        defaults,
      },
    });
  }
  return findings;
}

export function providerHealthFinding(
  project: DoctorProject,
  providerName: string,
  result: { status: number; body: OpenRouterHealthResponse | null },
): DoctorFinding | null {
  const unauthorized = result.status === 401 || result.status === 403;
  const unhealthy = result.status === 200 && result.body?.ok === false;
  if (!unauthorized && !unhealthy) return null;
  return {
    id: `provider_health:${project.projectId}:${providerName}`,
    severity: "warning",
    message: `Provider ${providerName} failed its health check on ${project.projectSlug}`,
    detail: unauthorized
      ? "Authentication failed. Update the provider credentials in the project settings."
      : result.body?.message ??
        "The provider reported an unhealthy status. Review its configuration.",
    fix: {
      kind: "open_project_providers",
      projectId: project.projectId,
      projectSlug: project.projectSlug,
    },
  };
}

export function quotaFinding(
  project: DoctorProject,
  providerName: string,
  info: OpenRouterAccountInfoResponse,
): DoctorFinding | null {
  const { limit, remaining, usage } = info.data;
  const exhausted =
    (typeof remaining === "number" && remaining <= 0) ||
    (typeof limit === "number" &&
      typeof usage === "number" &&
      limit > 0 &&
      usage >= limit);
  if (!exhausted) return null;
  return {
    id: `quota_429:${project.projectId}:${providerName}`,
    severity: "warning",
    message: `Token plan exhausted for ${providerName} on ${project.projectSlug}`,
    detail:
      "The provider account has no remaining credits. Requests will fail with 429 until the plan is upgraded or credits are added.",
    fix: {
      kind: "open_project_providers",
      projectId: project.projectId,
      projectSlug: project.projectSlug,
    },
  };
}

export function checkFailedFinding(
  project: DoctorProject,
  checkName: string,
): DoctorFinding {
  return {
    id: `check_failed:${project.projectId}:${checkName}`,
    severity: "info",
    message: `Check failed: ${checkName} on ${project.projectSlug}`,
    detail:
      "The check could not complete. The project worker may be busy or unreachable — run checks again.",
    fix: null,
  };
}

function normalizeSeverity(severity: string): DoctorSeverity {
  return severity === "critical" || severity === "warning" ? severity : "info";
}

export async function fetchServerFindings(
  daemonBase: string,
  signal?: AbortSignal,
): Promise<DoctorFinding[]> {
  const data = await fetchJson(`${daemonBase}/daemon/v1/doctor`, signal);
  const findings =
    data &&
    typeof data === "object" &&
    "findings" in data &&
    Array.isArray(data.findings)
      ? (data.findings as ServerFinding[])
      : [];
  return findings.map((finding) => ({
    id: `server:${finding.id}`,
    severity: normalizeSeverity(finding.severity),
    message: finding.message,
    detail: finding.detail ?? null,
    fix: resolveServerFixAction(finding.fix_action),
  }));
}

async function checkProject(
  daemonBase: string,
  worker: DaemonWorker,
  findings: DoctorFinding[],
  signal?: AbortSignal,
): Promise<void> {
  const project: DoctorProject = {
    projectId: worker.project_id,
    projectSlug: worker.slug,
  };
  let providers: ProviderListItem[] = [];
  try {
    providers = providerItems(
      await fetchJson(
        projectApiUrl(daemonBase, project.projectId, "/providers"),
        signal,
      ),
    );
  } catch {
    if (!signal?.aborted) {
      findings.push(checkFailedFinding(project, "provider list"));
    }
    return;
  }
  const configured = providers.filter(
    (provider) =>
      provider.status === "configured" || provider.status === "active",
  );

  try {
    const defaults = (await fetchJson(
      projectApiUrl(daemonBase, project.projectId, "/defaults"),
      signal,
    )) as ProviderDefaults;
    const availableModels: string[] = [];
    let modelListingFailed = false;
    for (const provider of configured.slice(0, MAX_MODEL_PROVIDERS)) {
      try {
        const data = await fetchJson(
          projectApiUrl(
            daemonBase,
            project.projectId,
            `/models?provider-name=${encodeURIComponent(provider.name)}`,
          ),
          signal,
        );
        availableModels.push(...collectProviderModels(provider.name, data));
      } catch {
        modelListingFailed = true;
      }
    }
    if (modelListingFailed) {
      if (!signal?.aborted) {
        findings.push(checkFailedFinding(project, "default models"));
      }
    } else {
      findings.push(
        ...staleDefaultModelFindings(project, defaults, availableModels),
      );
    }
  } catch {
    if (!signal?.aborted) {
      findings.push(checkFailedFinding(project, "default models"));
    }
  }

  const healthProviders = configured
    .filter((provider) =>
      HEALTH_CAPABLE_BASE_PROVIDERS.has(provider.base_provider),
    )
    .slice(0, MAX_HEALTH_CHECKS_PER_PROJECT);
  for (const provider of healthProviders) {
    try {
      const response = await fetchWithTimeout(
        projectApiUrl(
          daemonBase,
          project.projectId,
          `/providers/${encodeURIComponent(provider.name)}/health`,
        ),
        signal,
      );
      let body: OpenRouterHealthResponse | null = null;
      try {
        body = (await response.json()) as OpenRouterHealthResponse;
      } catch {
        body = null;
      }
      const finding = providerHealthFinding(project, provider.name, {
        status: response.status,
        body,
      });
      if (finding) findings.push(finding);
    } catch {
      if (!signal?.aborted) {
        findings.push(
          checkFailedFinding(project, `${provider.name} provider health`),
        );
      }
    }
  }

  if (configured.some((provider) => provider.base_provider === "openrouter")) {
    try {
      const info = (await fetchJson(
        projectApiUrl(
          daemonBase,
          project.projectId,
          "/openrouter/account-info",
        ),
        signal,
      )) as OpenRouterAccountInfoResponse;
      const finding = quotaFinding(project, "openrouter", info);
      if (finding) findings.push(finding);
    } catch {
      return;
    }
  }
}

export async function runClientChecks(
  daemonBase: string,
  workers: DaemonWorker[],
  signal?: AbortSignal,
): Promise<DoctorFinding[]> {
  const projects = workers.filter(isReadyWorker);
  const findings: DoctorFinding[] = [];
  let nextIndex = 0;

  async function runWorker() {
    while (nextIndex < projects.length && !signal?.aborted) {
      const worker = projects[nextIndex];
      nextIndex += 1;
      await checkProject(daemonBase, worker, findings, signal);
    }
  }

  await Promise.all(
    Array.from(
      { length: Math.min(MAX_CONCURRENT_PROJECTS, projects.length) },
      runWorker,
    ),
  );
  return findings;
}
