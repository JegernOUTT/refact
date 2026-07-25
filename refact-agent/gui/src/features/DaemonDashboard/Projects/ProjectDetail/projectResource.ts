import { useCallback, useEffect, useState } from "react";

import { projectApiUrl } from "../../../../services/refact/daemon";

const REQUEST_TIMEOUT_MS = 3_000;
export const CODE_INTEL_REQUEST_TIMEOUT_MS = 20_000;

export class ProjectRequestTimeoutError extends Error {
  constructor() {
    super("Project request timed out");
    this.name = "ProjectRequestTimeoutError";
  }
}

function requestTimeoutMs(path: string): number {
  return path.startsWith("/code-intel/")
    ? CODE_INTEL_REQUEST_TIMEOUT_MS
    : REQUEST_TIMEOUT_MS;
}

export type ProjectResource<T> =
  | { state: "loading" }
  | { state: "error"; kind: "failed" | "timeout" }
  | { state: "ready"; data: T };

export async function fetchProjectJson(
  daemonBase: string,
  projectId: string,
  path: string,
  signal?: AbortSignal,
  timeoutMs = requestTimeoutMs(path),
): Promise<unknown> {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => {
    controller.abort(new ProjectRequestTimeoutError());
  }, timeoutMs);
  const abort = () => controller.abort();
  signal?.addEventListener("abort", abort, { once: true });
  try {
    const response = await fetch(projectApiUrl(daemonBase, projectId, path), {
      credentials: "same-origin",
      signal: controller.signal,
    });
    if (!response.ok) throw new Error("Request failed");
    return (await response.json()) as unknown;
  } catch (error) {
    if (controller.signal.reason instanceof ProjectRequestTimeoutError) {
      throw controller.signal.reason;
    }
    throw error;
  } finally {
    signal?.removeEventListener("abort", abort);
    window.clearTimeout(timeout);
  }
}

export function useProjectResource<T>(
  daemonBase: string,
  projectId: string,
  path: string,
  parse: (data: unknown) => T | null,
  timeoutMs = requestTimeoutMs(path),
): { resource: ProjectResource<T>; refetch: () => void } {
  const [resource, setResource] = useState<ProjectResource<T>>({
    state: "loading",
  });
  const [generation, setGeneration] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    setResource({ state: "loading" });
    fetchProjectJson(daemonBase, projectId, path, controller.signal, timeoutMs)
      .then((data) => {
        if (!active) return;
        const parsed = parse(data);
        setResource(
          parsed === null
            ? { state: "error", kind: "failed" }
            : { state: "ready", data: parsed },
        );
      })
      .catch((error: unknown) => {
        if (!active) return;
        setResource({
          state: "error",
          kind:
            error instanceof ProjectRequestTimeoutError ? "timeout" : "failed",
        });
      });
    return () => {
      active = false;
      controller.abort();
    };
  }, [daemonBase, generation, parse, path, projectId, timeoutMs]);

  const refetch = useCallback(() => {
    setGeneration((current) => current + 1);
  }, []);

  return { resource, refetch };
}

export function codeIntelData<T extends object>(data: unknown): T | null {
  if (!data || typeof data !== "object") return null;
  if ("detail" in data) return null;
  return data as T;
}
