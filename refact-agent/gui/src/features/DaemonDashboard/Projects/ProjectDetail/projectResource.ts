import { useCallback, useEffect, useState } from "react";

import { projectApiUrl } from "../../../../services/refact/daemon";

const REQUEST_TIMEOUT_MS = 3_000;

export type ProjectResource<T> =
  | { state: "loading" }
  | { state: "error" }
  | { state: "ready"; data: T };

export async function fetchProjectJson(
  daemonBase: string,
  projectId: string,
  path: string,
  signal?: AbortSignal,
): Promise<unknown> {
  const controller = new AbortController();
  const timeout = window.setTimeout(
    () => controller.abort(),
    REQUEST_TIMEOUT_MS,
  );
  const abort = () => controller.abort();
  signal?.addEventListener("abort", abort, { once: true });
  try {
    const response = await fetch(projectApiUrl(daemonBase, projectId, path), {
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

export function useProjectResource<T>(
  daemonBase: string,
  projectId: string,
  path: string,
  parse: (data: unknown) => T | null,
): { resource: ProjectResource<T>; refetch: () => void } {
  const [resource, setResource] = useState<ProjectResource<T>>({
    state: "loading",
  });
  const [generation, setGeneration] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    let active = true;
    setResource({ state: "loading" });
    fetchProjectJson(daemonBase, projectId, path, controller.signal)
      .then((data) => {
        if (!active) return;
        const parsed = parse(data);
        setResource(
          parsed === null
            ? { state: "error" }
            : { state: "ready", data: parsed },
        );
      })
      .catch(() => {
        if (active) setResource({ state: "error" });
      });
    return () => {
      active = false;
      controller.abort();
    };
  }, [daemonBase, generation, parse, path, projectId]);

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
