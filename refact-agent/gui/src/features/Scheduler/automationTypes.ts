// Wire boundary for the project Automation facade. Unknown variants stay read-only.
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
export const isRevision = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;

export type AutomationDefinition = {
  schema_version: number;
  id: string;
  revision: number;
  owner: Record<string, unknown> & { kind: string };
  description: string;
  enabled: boolean;
  trigger: Record<string, unknown> & { kind: string };
  action: Record<string, unknown> & { kind: string };
  deliveries: Record<string, unknown>[];
  lifecycle: Record<string, unknown> & { kind: string };
  policy: Record<string, unknown>;
};
const tagged = (
  value: unknown,
): value is Record<string, unknown> & { kind: string } =>
  isRecord(value) && typeof value.kind === "string";
export function isAutomationDefinition(
  value: unknown,
): value is AutomationDefinition {
  return (
    isRecord(value) &&
    isRevision(value.schema_version) &&
    typeof value.id === "string" &&
    isRevision(value.revision) &&
    value.revision > 0 &&
    tagged(value.owner) &&
    typeof value.description === "string" &&
    typeof value.enabled === "boolean" &&
    tagged(value.trigger) &&
    tagged(value.action) &&
    Array.isArray(value.deliveries) &&
    value.deliveries.every(isRecord) &&
    tagged(value.lifecycle) &&
    isRecord(value.policy)
  );
}
export type AutomationEntry = {
  definition: AutomationDefinition;
  status: {
    authorization: string | null;
    execution: Record<string, unknown> | null;
    freshness: Record<string, unknown> | null;
    delivery: Record<string, unknown> | null;
  };
  project_wake?: {
    next_wake_ms: number | null;
    background_active: boolean;
    pending_delivery: number;
    recovery: number;
  };
  capabilities?: { desired_control: string[] };
};
export type AutomationCatalog = {
  catalog_revision: number;
  definitions: AutomationEntry[];
};
export function isAutomationEntry(value: unknown): value is AutomationEntry {
  if (
    !isRecord(value) ||
    !isAutomationDefinition(value.definition) ||
    !isRecord(value.status)
  )
    return false;
  if (value.project_wake !== undefined) {
    const wake = value.project_wake;
    if (
      !isRecord(wake) ||
      !(wake.next_wake_ms === null || isRevision(wake.next_wake_ms)) ||
      typeof wake.background_active !== "boolean" ||
      !isRevision(wake.pending_delivery) ||
      !isRevision(wake.recovery)
    )
      return false;
  }
  const status = value.status;
  if (
    !(status.authorization === null || typeof status.authorization === "string")
  )
    return false;
  if (
    ![status.execution, status.freshness, status.delivery].every(
      (v) => v === null || isRecord(v),
    )
  )
    return false;
  return (
    value.capabilities === undefined ||
    (isRecord(value.capabilities) &&
      Array.isArray(value.capabilities.desired_control) &&
      value.capabilities.desired_control.every((v) => typeof v === "string"))
  );
}
export function isAutomationCatalog(
  value: unknown,
): value is AutomationCatalog {
  return (
    isRecord(value) &&
    isRevision(value.catalog_revision) &&
    Array.isArray(value.definitions) &&
    value.definitions.every(isAutomationEntry)
  );
}
export type AutomationView = "schedule" | "continuous" | "channel_event";
export function supportsDefinition(definition: AutomationDefinition): boolean {
  if (
    definition.action.kind === "collect" &&
    (!isRecord(definition.action.driver) ||
      typeof definition.action.driver.type !== "string" ||
      ![
        "manual",
        "file_tail",
        "interval",
        "http_poll",
        "sse",
        "websocket",
        "process",
        "internal",
      ].includes(definition.action.driver.type))
  )
    return false;
  return (
    definition.schema_version === 1 &&
    definition.owner.kind === "project" &&
    ["schedule", "continuous", "channel_event"].includes(
      definition.trigger.kind,
    ) &&
    ["legacy", "collect", "transform"].includes(definition.action.kind)
  );
}
// Reject inline credentials before anything can enter RTK Query actions or traces.
function isCredentialBinding(value: unknown): boolean {
  return (
    isRecord(value) &&
    Object.keys(value).every((key) =>
      ["reference", "header", "scheme", "expected_identity_revision"].includes(
        key,
      ),
    ) &&
    typeof value.reference === "string" &&
    /^[A-Za-z0-9_.:-]{1,256}$/.test(value.reference) &&
    typeof value.header === "string" &&
    /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(value.header) &&
    (value.scheme === undefined ||
      value.scheme === null ||
      (typeof value.scheme === "string" &&
        /^[A-Za-z0-9_-]{1,32}$/.test(value.scheme))) &&
    (value.expected_identity_revision === undefined ||
      value.expected_identity_revision === null ||
      (typeof value.expected_identity_revision === "string" &&
        /^[A-Za-z0-9_.:-]{1,256}$/.test(value.expected_identity_revision)))
  );
}
export function containsInlineCredential(value: unknown): boolean {
  function visit(child: unknown, path: string[]): boolean {
    if (typeof child === "string") {
      if (/^\s*(?:https?|wss?):\/\//i.test(child)) {
        try {
          const url = new URL(child);
          if (url.username || url.password || url.search || url.hash)
            return true;
        } catch {
          return true;
        }
      }
      return /(?:https?|wss?):\/\/[^/\s]*@|[?&](?:token|key|secret|api_key)=/i.test(
        child,
      );
    }
    if (Array.isArray(child)) return child.some((item) => visit(item, path));
    if (!isRecord(child)) return false;
    if (typeof child.url === "string") {
      try {
        const url = new URL(child.url);
        if (url.username || url.password || url.search || url.hash) return true;
      } catch {
        return true;
      }
    }
    return Object.entries(child).some(([key, item]) => {
      if (
        key === "credential_bindings" &&
        path.join(".") === "action.policy" &&
        isRecord(value) &&
        isRecord(value.action) &&
        value.action.kind === "collect"
      )
        return (
          !Array.isArray(item) ||
          item.length > 16 ||
          !item.every(isCredentialBinding)
        );
      return (
        /(?:token|secret|password|authorization|headers|^env$|api.?key|bearer|credential(?!_ref)|signature)/i.test(
          key,
        ) || visit(item, [...path, key])
      );
    });
  }
  return visit(value, []);
}
export function parseDefinitionDraft(text: string): AutomationDefinition {
  if (text.length > 65536)
    throw new Error("Definition must be at most 64 KiB.");
  const value: unknown = JSON.parse(text);
  if (!isAutomationDefinition(value))
    throw new Error(
      "Provide a complete Automation definition with valid field types.",
    );
  if (!supportsDefinition(value))
    throw new Error(
      "Unsupported schema, owner, trigger or action. App definitions are managed through their manifest.",
    );
  if (containsInlineCredential(value))
    throw new Error(
      "Inline credentials, headers and environment values are not accepted. Use credential_ref metadata and the trusted host credential flow.",
    );
  if (value.action.kind === "collect") {
    const policy = value.action.policy;
    if (isRecord(policy) && "owner_definition_digest" in policy)
      throw new Error("Owner definition digest is host-only.");
    const driver = value.action.driver;
    const driverFields: Record<string, string[]> = {
      manual: [],
      file_tail: ["path", "every_ms"],
      interval: ["every_ms"],
      http_poll: ["url", "every_ms"],
      sse: ["url"],
      websocket: ["url"],
      process: ["process_id", "chat_id"],
      internal: ["topic"],
    };
    const fields =
      isRecord(driver) && typeof driver.type === "string"
        ? driverFields[driver.type]
        : undefined;
    if (
      !isRecord(driver) ||
      !fields ||
      Object.keys(driver).some((key) => key !== "type" && !fields.includes(key))
    )
      throw new Error("Unsupported collector driver fields.");
  }
  if (!/^[A-Za-z0-9_.:-]{1,256}$/.test(value.id))
    throw new Error(
      "Use 1–256 letters, digits, underscores, dots, colons or hyphens for the ID.",
    );
  return value;
}
