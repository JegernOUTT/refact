import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";

import type { EngineApiConfig } from "./apiUrl";
import { daemonApi, resolveDaemonBaseUrl } from "./daemon";

export type Observation<T> =
  | { state: "unknown" }
  | { state: "known"; value: T };

export type RuntimeChannel = "stable" | "preview" | "nightly";

export type RuntimeRepository = { owner: string; name: string; id: number };
export type RuntimeWorkflow = { path: string; id: number };
export type RuntimeRun = { id: number; attempt: number };

export type PinnedSource =
  | { kind: "release"; tag: string }
  | { kind: "run"; id: number; attempt: number };

export type RuntimeSelection =
  | {
      kind: "channel";
      repository: RuntimeRepository;
      channel: RuntimeChannel;
      workflow?: RuntimeWorkflow;
    }
  | {
      kind: "pinned";
      repository: RuntimeRepository;
      source: PinnedSource;
      expected_sha?: string;
      manifest_digest?: string;
    }
  | { kind: "local"; absolute_path: string };

export type ComponentVersions = {
  runtime: string;
  gui: string;
  vscode: string;
  jetbrains: string;
};

export type ManagedIdentity = {
  manifest_digest: string;
  target: string;
  component_versions: ComponentVersions;
};

export type InstalledRuntime = {
  absolute_path: string;
  identity: ManagedIdentity;
};

export type RunningProcess = {
  pid: number;
  reported_versions: Observation<ComponentVersions>;
};

export type RuntimeStatus = {
  selection_revision: number;
  selected: Observation<RuntimeSelection>;
  installed: Observation<InstalledRuntime>;
  staged: Observation<ManagedIdentity>;
  running: Observation<RunningProcess>;
};

export type RuntimeStatusResponse = {
  status: RuntimeStatus;
  target: string;
  diagnostics: string[];
  activation_supported: boolean;
  restart_dispatched: boolean;
};

export type SourceEvidence = {
  repository: RuntimeRepository;
  workflow: RuntimeWorkflow;
  run: RuntimeRun;
  source_sha: string;
  status: string;
  conclusion: string;
};

export type ManifestTestResult = "passed" | "failed" | "not_run";

export type RuntimeUpdatePlan = {
  selection_revision: number;
  identity: ManagedIdentity;
  source: SourceEvidence;
  manifest_tests: ManifestTestResult;
  qualification: string;
  public_qualification_verified: boolean;
  together: string[];
  ide_owned_not_updated: string[];
  activation_supported: boolean;
  restart_dispatched: boolean;
};

export type RuntimeStageInfo = {
  selection_revision: number;
  manifest_digest: string;
  target: string;
  runtime_sha256: string;
  runtime_size: number;
};

export type RuntimeStageResponse = {
  plan: RuntimeUpdatePlan;
  staged: RuntimeStageInfo;
};

export type RuntimeSelectionSnapshot = {
  schema_version: number;
  revision: number;
  selection: RuntimeSelection;
};

export type RuntimeStageKey = {
  manifest_digest: string;
  selection_revision: number;
};

export type RuntimeSelectRequest = {
  expected_revision: number;
  selection: RuntimeSelection;
};

export type RuntimeRevisionRequest = { expected_revision: number };

export const RUNTIME_REPOSITORY: RuntimeRepository = {
  owner: "JegernOUTT",
  name: "refact",
  id: 1254216370,
};

const GITHUB_PREFIX = "https://github.com/";
const RUNTIME_TAG = "RuntimeCoordinator" as const;

function hasControlOrSpace(value: string): boolean {
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0;
    if (code <= 0x20 || (code >= 0x7f && code <= 0x9f) || /\s/u.test(char)) {
      return true;
    }
  }
  return false;
}

function positiveInteger(value: string): number | null {
  if (!/^[0-9]+$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
}

function validTag(tag: string): boolean {
  return (
    tag.length > 0 &&
    new TextEncoder().encode(tag).length <= 200 &&
    !hasControlOrSpace(tag) &&
    !/[\\:]/.test(tag) &&
    tag.split("/").every((part) => part && part !== "." && part !== "..")
  );
}

export function parseRuntimeSource(
  input: string,
  channel: RuntimeChannel,
): RuntimeSelection | null {
  if (hasControlOrSpace(input) || input.includes("\\")) return null;
  if (!input.startsWith(GITHUB_PREFIX) || /[?#]/.test(input)) return null;
  let url: URL;
  try {
    url = new URL(input);
  } catch {
    return null;
  }
  if (
    url.protocol !== "https:" ||
    url.hostname !== "github.com" ||
    url.username ||
    url.password ||
    url.port
  ) {
    return null;
  }
  const parts: string[] = [];
  for (const raw of input.slice(GITHUB_PREFIX.length).split("/")) {
    let part: string;
    try {
      part = decodeURIComponent(raw);
    } catch {
      return null;
    }
    if (
      !part ||
      part === "." ||
      part === ".." ||
      /[/\\:]/.test(part) ||
      hasControlOrSpace(part)
    ) {
      return null;
    }
    parts.push(part);
  }
  if (
    parts.length < 2 ||
    parts[0] !== RUNTIME_REPOSITORY.owner ||
    parts[1] !== RUNTIME_REPOSITORY.name
  ) {
    return null;
  }
  const repository = { ...RUNTIME_REPOSITORY };
  if (parts.length === 2 || (parts.length === 3 && parts[2] === "releases")) {
    return { kind: "channel", repository, channel };
  }
  if (parts[2] === "releases" && parts.length >= 5 && parts[3] === "tag") {
    const tag = parts.slice(4).join("/");
    return validTag(tag)
      ? { kind: "pinned", repository, source: { kind: "release", tag } }
      : null;
  }
  if (
    parts[2] === "actions" &&
    parts[3] === "runs" &&
    (parts.length === 5 || (parts.length === 7 && parts[5] === "attempts"))
  ) {
    const id = positiveInteger(parts[4]);
    const attempt = parts.length === 7 ? positiveInteger(parts[6]) : 1;
    return id && attempt
      ? { kind: "pinned", repository, source: { kind: "run", id, attempt } }
      : null;
  }
  return null;
}

export function runtimeStatusUrl(
  root: string,
  stageKey?: RuntimeStageKey | null,
): string {
  const base = `${root}/daemon/v1/runtime/status`;
  if (!stageKey) return base;
  const params = new URLSearchParams({
    manifest_digest: stageKey.manifest_digest,
    selection_revision: String(stageKey.selection_revision),
  });
  return `${base}?${params.toString()}`;
}

function invalidateOnConflict(
  _result: unknown,
  error: FetchBaseQueryError | undefined,
): (typeof RUNTIME_TAG)[] {
  return error?.status === 409 ? [RUNTIME_TAG] : [];
}

export const runtimeCoordinatorApi = daemonApi
  .enhanceEndpoints({ addTagTypes: [RUNTIME_TAG] })
  .injectEndpoints({
    endpoints: (builder) => ({
      getRuntimeStatus: builder.query<
        RuntimeStatusResponse,
        RuntimeStageKey | undefined
      >({
        providesTags: [RUNTIME_TAG],
        queryFn: async (stageKey, api, _opts, baseQuery) => {
          const { config } = api.getState() as { config: EngineApiConfig };
          const result = await baseQuery(
            runtimeStatusUrl(resolveDaemonBaseUrl(config), stageKey),
          );
          return result.error
            ? { error: result.error }
            : { data: result.data as RuntimeStatusResponse };
        },
      }),
      selectRuntime: builder.mutation<
        RuntimeSelectionSnapshot,
        RuntimeSelectRequest
      >({
        invalidatesTags: (_result, error) =>
          !error || error.status === 409 ? [RUNTIME_TAG] : [],
        queryFn: async (body, api, _opts, baseQuery) => {
          const { config } = api.getState() as { config: EngineApiConfig };
          const result = await baseQuery({
            url: `${resolveDaemonBaseUrl(config)}/daemon/v1/runtime/select`,
            method: "POST",
            body,
          });
          return result.error
            ? { error: result.error }
            : { data: result.data as RuntimeSelectionSnapshot };
        },
      }),
      resolveRuntime: builder.mutation<
        RuntimeUpdatePlan,
        RuntimeRevisionRequest
      >({
        invalidatesTags: invalidateOnConflict,
        queryFn: async (body, api, _opts, baseQuery) => {
          const { config } = api.getState() as { config: EngineApiConfig };
          const result = await baseQuery({
            url: `${resolveDaemonBaseUrl(config)}/daemon/v1/runtime/resolve`,
            method: "POST",
            body,
          });
          return result.error
            ? { error: result.error }
            : { data: result.data as RuntimeUpdatePlan };
        },
      }),
      stageRuntime: builder.mutation<
        RuntimeStageResponse,
        RuntimeRevisionRequest
      >({
        invalidatesTags: invalidateOnConflict,
        queryFn: async (body, api, _opts, baseQuery) => {
          const { config } = api.getState() as { config: EngineApiConfig };
          const result = await baseQuery({
            url: `${resolveDaemonBaseUrl(config)}/daemon/v1/runtime/stage`,
            method: "POST",
            body,
          });
          return result.error
            ? { error: result.error }
            : { data: result.data as RuntimeStageResponse };
        },
      }),
    }),
  });

export const {
  useGetRuntimeStatusQuery,
  useSelectRuntimeMutation,
  useResolveRuntimeMutation,
  useStageRuntimeMutation,
} = runtimeCoordinatorApi;
