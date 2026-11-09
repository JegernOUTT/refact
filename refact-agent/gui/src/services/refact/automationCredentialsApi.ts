// Deliberately NOT an RTK API: secret values must never become Redux action args.
export type CredentialIdentity = {
  id: string;
  origin: string;
  header: string;
  tenant: string;
  principal: string;
  scope: string[];
};
export type CredentialMetadata = CredentialIdentity & {
  identity_revision: string;
  secret_revision: string;
};
export type CredentialCatalog = {
  supported: boolean;
  project_root: string;
  credentials: CredentialMetadata[];
};
export type CredentialMutation =
  | {
      operation: "create";
      project_root: string;
      identity: CredentialIdentity;
      secret: string;
    }
  | {
      operation: "rotate";
      project_root: string;
      id: string;
      expected_secret_revision: string;
      secret: string;
    }
  | {
      operation: "remove";
      project_root: string;
      id: string;
      expected_secret_revision: string;
    };

export function credentialHostAvailable(): boolean {
  // Only the worker's own top-level HTTP UI, never an opaque App frame, IDE
  // webview, remote URL, arbitrary configurable backend, or original :8001.
  return (
    window.location.protocol === "http:" &&
    ["localhost", "127.0.0.1", "[::1]"].includes(window.location.hostname) &&
    window.parent === window
  );
}
function endpoint(path: string): string {
  if (!credentialHostAvailable())
    throw new Error("Open the trusted local worker UI to manage credentials.");
  return new URL(`/v1/scheduler/credentials/${path}`, window.location.origin)
    .href;
}
function headers(token: string | null | undefined): Headers {
  const result = new Headers({ "Content-Type": "application/json" });
  if (token) result.set("Authorization", `Bearer ${token}`);
  return result;
}
function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}
function metadata(value: unknown): value is CredentialMetadata {
  return (
    isRecord(value) &&
    [
      "id",
      "origin",
      "header",
      "tenant",
      "principal",
      "identity_revision",
      "secret_revision",
    ].every((key) => typeof value[key] === "string") &&
    Array.isArray(value.scope) &&
    value.scope.every((s: unknown) => typeof s === "string")
  );
}
function checked(response: Response): void {
  // Never echo an untrusted error response or request (which may contain a secret).
  if (!response.ok)
    throw new Error(
      response.status === 409
        ? "Credential or project changed. Refresh metadata before trying again."
        : response.status === 501
          ? "Private credential enrollment is unsupported on this platform."
          : "Credential request refused. Check trusted host access, identity, value limits, and private storage.",
    );
}
export async function listCredentials(
  token?: string | null,
): Promise<CredentialCatalog> {
  const response = await fetch(endpoint(""), {
    headers: headers(token),
    cache: "no-store",
    credentials: "same-origin",
    mode: "same-origin",
    redirect: "error",
  });
  checked(response);
  const value: unknown = await response.json();
  if (
    !isRecord(value) ||
    typeof value.supported !== "boolean" ||
    typeof value.project_root !== "string" ||
    !Array.isArray(value.credentials) ||
    !value.credentials.every(metadata)
  ) {
    throw new Error(
      "Unsupported credential metadata response. Update the worker.",
    );
  }
  // Explicit allowlist: never retain unexpected server properties.
  return {
    supported: value.supported,
    project_root: value.project_root,
    credentials: value.credentials.map((item) => ({
      id: item.id,
      origin: item.origin,
      header: item.header,
      tenant: item.tenant,
      principal: item.principal,
      scope: item.scope,
      identity_revision: item.identity_revision,
      secret_revision: item.secret_revision,
    })),
  };
}
export async function mutateCredential(
  request: CredentialMutation,
  token?: string | null,
): Promise<void> {
  const { operation, ...body } = request;
  const response = await fetch(endpoint(operation), {
    method: "POST",
    headers: headers(token),
    body: JSON.stringify(body),
    cache: "no-store",
    credentials: "same-origin",
    mode: "same-origin",
    redirect: "error",
  });
  checked(response);
  // The mutation response is intentionally not retained in state or Redux.
}
