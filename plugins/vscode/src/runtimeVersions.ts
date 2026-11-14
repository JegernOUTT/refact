export type RuntimeVersionObservation = {
    state: "observed" | "last_observed";
    version?: string | null;
    commit?: string | null;
    source?: "plugin" | "local" | "daemon" | "worker";
    observedAtMs?: number;
};

export type RuntimeVersions = {
    plugin?: RuntimeVersionObservation | null;
    selected?: RuntimeVersionObservation | null;
    installed?: RuntimeVersionObservation | null;
    staged?: RuntimeVersionObservation | null;
    daemon?: RuntimeVersionObservation | null;
    worker?: RuntimeVersionObservation | null;
    requestedVersion?: string | null;
};

export function observedVersion(source: RuntimeVersionObservation["source"], version?: string | null, commit?: string | null): RuntimeVersionObservation {
    return { state: "observed", version: cleanString(version), commit: cleanString(commit), source, observedAtMs: Date.now() };
}

function cleanString(value: unknown): string | null {
    return typeof value === "string" && value.trim() && value.trim().length <= 160 && !/[\u0000-\u001f\u007f]/.test(value) ? value.trim() : null;
}

export function emptyRuntimeVersions(pluginVersion: unknown): RuntimeVersions {
    const version = cleanString(pluginVersion);
    return { plugin: version ? observedVersion("plugin", version) : null, requestedVersion: version, selected: null, installed: null, staged: null, daemon: null, worker: null };
}

export function workerBuildObservation(value: unknown): RuntimeVersionObservation | null {
    if (!value || typeof value !== "object" || Array.isArray(value)) { return null; }
    const info = value as { version?: unknown; commit?: unknown };
    const version = cleanString(info.version);
    const commit = cleanString(info.commit);
    return version || commit ? observedVersion("worker", version, commit) : null;
}

export function lastObserved(value: RuntimeVersionObservation | null | undefined): RuntimeVersionObservation | null {
    return value ? { ...value, state: "last_observed" } : null;
}
