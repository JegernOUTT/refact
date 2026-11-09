import { normalizeToolName } from "../../../utils/toolNameAliases";
import { humanizeIdentifier } from "../../../utils/displayNames";
import type { TranscriptStatus } from "../TranscriptItem";

export const APP_TOOL_NAMES = [
  "app",
  "app_build",
  "app_state",
  "app_binding",
  "app_layout",
  "app_view",
  "app_source",
  "app_event",
  "app_inspect",
  "app_screenshot",
] as const;
export type AppToolName = (typeof APP_TOOL_NAMES)[number];
type RecordValue = Record<string, unknown>;
export interface AppFact {
  label: string;
  value: string;
}
export interface AppSection {
  title: string;
  rows: AppFact[];
}
export interface AppToolModel {
  title: string;
  target?: string;
  outcome: string;
  status: TranscriptStatus;
  facts: AppFact[];
  sections: AppSection[];
  images: string[];
  raw: unknown;
}
const record = (value: unknown): value is RecordValue =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const obj = (value: unknown): RecordValue => (record(value) ? value : {});
const scalar = (value: unknown): string | undefined =>
  typeof value === "string" ||
  typeof value === "number" ||
  typeof value === "boolean"
    ? String(value)
    : undefined;
export function isAppToolName(name: string | undefined): name is AppToolName {
  return APP_TOOL_NAMES.some((candidate) => candidate === name);
}
// Defense in depth for permitted responses, not a replacement for upstream privacy clearance.
export function sanitizeAppValue(value: unknown, depth = 0): unknown {
  if (depth > 12) return "[Preview depth limit]";
  if (typeof value === "string") {
    try {
      const decoded: unknown = JSON.parse(value);
      if (record(decoded) || Array.isArray(decoded))
        return sanitizeAppValue(decoded, depth + 1);
    } catch {
      /* ordinary text */
    }
    return value
      .replace(/(?:https?|wss?):\/\/[^\s<>"']+/gi, (url) => {
        try {
          return new URL(url).origin;
        } catch {
          return "[Endpoint omitted]";
        }
      })
      .replace(/data:image\/[^\s"']+/gi, "[Image artifact omitted]")
      .replace(
        /((?:[\w-]*token|api[_-]?key|authorization|password|secret)["'\s]*[:=]["'\s]*)[^\s,;"'<>]+/gi,
        "$1[Redacted]",
      );
  }
  if (Array.isArray(value))
    return value
      .slice(0, 200)
      .map((item: unknown) => sanitizeAppValue(item, depth + 1));
  if (record(value))
    return Object.fromEntries(
      Object.entries(value)
        .slice(0, 200)
        .map(([key, item]) => [
          key,
          /token|secret|password|authorization|cookie|api[_-]?key|credential/i.test(
            key,
          )
            ? "[Redacted]"
            : key === "data_url" || key === "icon_artwork"
              ? "[Artifact omitted]"
              : sanitizeAppValue(item, depth + 1),
        ]),
    );
  return value;
}
function text(value: unknown): string {
  const clean = sanitizeAppValue(value);
  const simple = scalar(clean);
  if (simple !== undefined) return simple.slice(0, 1200);
  if (Array.isArray(clean)) return `${clean.length} items`;
  if (record(clean))
    return Object.keys(clean).slice(0, 12).join(", ") || "Empty object";
  return clean === null ? "None" : "Not recorded";
}
function facts(value: RecordValue, keys: readonly string[]): AppFact[] {
  return keys
    .filter((key) => value[key] !== undefined)
    .map((key) => ({
      label: humanizeIdentifier(key),
      value: text(value[key]),
    }));
}
function parse(content: unknown): {
  value: unknown;
  images: string[];
  svg: boolean;
} {
  const images: string[] = [];
  let input = content;
  if (
    Array.isArray(content) &&
    content.some(
      (item: unknown) => record(item) && typeof item.m_type === "string",
    )
  ) {
    const parts: string[] = [];
    for (const item of content as unknown[]) {
      if (!record(item) || typeof item.m_content !== "string") continue;
      if (item.m_type === "text") parts.push(item.m_content);
      // Never load returned network URLs or execute an SVG. Only raster base64 parts.
      if (
        item.m_type === "image/png" &&
        /^[A-Za-z0-9+/]+={0,2}$/.test(item.m_content)
      )
        images.push(`data:image/png;base64,${item.m_content}`);
    }
    input = parts.join("\n");
  }
  const svg =
    typeof input === "string" &&
    /Capture artifact .*data:image\/svg\+xml/i.test(input);
  if (typeof input === "string") {
    try {
      input = JSON.parse(input) as unknown;
    } catch {
      if (svg) {
        try {
          const firstLine = String(input).split("\n")[0];
          input = JSON.parse(firstLine) as unknown;
        } catch {
          /* safe fallback below */
        }
      }
    }
  }
  return { value: input, images, svg };
}
const COMMON = [
  "app_id",
  "revision",
  "active_revision",
  "binding_id",
  "view_id",
  "generation",
  "status",
  "source_id",
  "channel",
  "seq",
  "trust",
  "method",
];
const ACTIONS = new Set([
  "help",
  "list",
  "get",
  "create",
  "read_file",
  "write_file",
  "rename",
  "clone",
  "archive",
  "remove",
  "status",
  "publish",
  "rollback",
  "set",
  "attach",
  "update",
  "detach",
  "open",
  "focus",
  "close",
  "read",
  "configure",
  "control",
  "grant_endpoint",
  "emit",
  "schema",
  "capture",
]);
function hasEnvelopeErrors(value: RecordValue): boolean {
  return (
    ["error", "last_error", "errors"].some((key) => {
      const item = value[key];
      return (
        item !== undefined &&
        item !== null &&
        item !== "" &&
        item !== false &&
        (!Array.isArray(item) || item.length > 0)
      );
    }) ||
    ["error", "failed", "unsupported", "permission_required"].includes(
      String(value.status),
    )
  );
}
export function buildAppToolModel(
  name: string | undefined,
  argumentsText: string,
  content: unknown,
  hasResult: boolean,
  failed: boolean,
): AppToolModel {
  const normalized = normalizeToolName(name);
  const family = isAppToolName(normalized) ? normalized : "app";
  let action = "";
  try {
    const args: unknown = JSON.parse(argumentsText);
    if (
      record(args) &&
      typeof args.action === "string" &&
      ACTIONS.has(args.action)
    )
      action = args.action;
  } catch {
    /* incomplete arguments */
  }
  const parsed = parse(content);
  const safeValue = sanitizeAppValue(parsed.value);
  const value = obj(safeValue);
  const app = obj(value.app);
  const metadata = obj(value.metadata);
  const manifest = obj(value.manifest ?? app.manifest);
  const resultFacts = facts(value, COMMON);
  const sections: AppSection[] = [];
  const add = (title: string, rows: AppFact[]) => {
    if (rows.length) sections.push({ title, rows });
  };
  const addCollection = (
    title: string,
    input: unknown,
    keys: readonly string[],
  ) => {
    if (!Array.isArray(input)) return;
    add(
      title,
      input.slice(0, 16).flatMap((item: unknown, index) =>
        record(item)
          ? [
              {
                label: `${title} ${index + 1}`,
                value: text(
                  item.name ??
                    obj(item.manifest).name ??
                    item.source_id ??
                    item.kind ??
                    item.view_id ??
                    item.binding_id ??
                    item.slug ??
                    index + 1,
                ),
              },
              ...facts(item, keys),
            ]
          : [{ label: `${index + 1}`, value: text(item) }],
      ),
    );
  };
  add(
    "App",
    facts(manifest, ["name", "description", "entrypoint", "schema_version"]),
  );
  add("Build", facts(app, ["revision", "active_revision", "slug"]));
  if (value.protocol_version !== undefined) {
    add("Protocol", facts(value, ["protocol_version", "sdk_version"]));
    for (const key of [
      "authoring",
      "sdk",
      "state_controls",
      "sources",
      "csp",
      "theme",
    ]) {
      const part = obj(value[key]);
      add(
        humanizeIdentifier(key),
        Object.entries(part)
          .slice(0, 12)
          .map(([label, item]) => ({
            label: humanizeIdentifier(label),
            value: Array.isArray(item) ? item.map(text).join("\n") : text(item),
          })),
      );
    }
  }
  add("Resource", facts(value, ["path", "content"]));
  addCollection("Files", value.files ?? app.files, []);
  addCollection("Apps", value.apps, ["slug", "revision", "active_revision"]);
  if (family === "app_state") {
    add("State", facts(value, ["value", "scope"]));
    if (record(value.value))
      add(
        "State fields",
        Object.entries(value.value)
          .slice(0, 20)
          .map(([label, item]) => ({ label, value: text(item) })),
      );
  }
  addCollection("Bindings", value.bindings, [
    "app_id",
    "binding_id",
    "params",
    "state",
  ]);
  const layoutRows: AppFact[] = [];
  const layout = (node: unknown, depth = 0): void => {
    if (!record(node) || depth > 8 || layoutRows.length > 40) return;
    layoutRows.push({
      label: depth ? "Pane" : "Layout",
      value:
        node.kind === "split"
          ? `${text(node.direction)} split · ${text(node.sizes)}`
          : `Tabs · active ${text(node.active)}`,
    });
    if (Array.isArray(node.tabs))
      for (const tab of node.tabs as unknown[])
        if (record(tab))
          layoutRows.push({
            label: "Tab",
            value:
              tab.kind === "chat"
                ? "Chat"
                : `App binding ${text(tab.binding_id)}`,
          });
    if (Array.isArray(node.children))
      node.children.forEach((child: unknown) => layout(child, depth + 1));
  };
  layout(value.layout);
  add("Workspace layout", layoutRows);
  const runtimeKeys = [
    "source_id",
    "channel",
    "status",
    "driver",
    "granted",
    "generation",
    "seq",
    "last_error",
    "error",
    "last_data",
    "last_good_data",
    "data",
  ];
  add("Source", facts(obj(value.source), runtimeKeys));
  if (family === "app_source") {
    add("Source runtime", facts(value, runtimeKeys));
    const driver = obj(obj(value.source).driver ?? value.driver);
    add(
      "Driver",
      facts(driver, [
        "type",
        "url",
        "every_ms",
        "process_id",
        "chat_id",
        "topic",
      ]),
    );
  }
  addCollection("Sources", value.sources, runtimeKeys);
  addCollection("Views", value.views, [
    "revision",
    "host",
    "visible",
    "focused",
    "ready",
    "stale",
    "generation",
  ]);
  addCollection("Events", value.events, ["seq", "kind", "channel", "data"]);
  addCollection(
    family === "app_view"
      ? "Views"
      : family === "app_event"
        ? "Events"
        : "Sources",
    safeValue,
    family === "app_view"
      ? ["app_id", "revision", "host", "visible", "focused", "ready", "stale"]
      : family === "app_event"
        ? ["seq", "kind", "data"]
        : runtimeKeys,
  );
  if (family === "app_event")
    add(
      "Event",
      facts(value, ["accepted", "kind", "data", "envelope", "capacity"]),
    );
  add(
    "Capture metadata (app-reported)",
    facts(metadata, [
      "width",
      "height",
      "device_pixel_ratio",
      "warnings",
      "limitations",
      "title",
      "url",
    ]),
  );
  add(
    "Diagnostics",
    facts(value, [
      "diagnostics_version",
      "limitations",
      "error",
      "last_error",
      "errors",
      "diagnostics",
    ]),
  );
  const withheld =
    typeof parsed.value === "string" &&
    parsed.value.startsWith(
      "App operation completed, but its response was withheld by post-operation privacy clearance:",
    );
  const errors =
    family === "app_state"
      ? false
      : hasEnvelopeErrors(obj(parsed.value)) ||
        (family === "app_source" &&
          Array.isArray(safeValue) &&
          safeValue.some((item: unknown) => hasEnvelopeErrors(obj(item)))) ||
        ((family === "app_source" || family === "app_inspect") &&
          Array.isArray(value.sources) &&
          value.sources.some((item: unknown) => hasEnvelopeErrors(obj(item))));
  const recognized =
    Array.isArray(parsed.value) ||
    resultFacts.length > 0 ||
    sections.length > 0 ||
    value.accepted === true ||
    value.removed === true ||
    value.archive_id !== undefined;
  let outcome = !hasResult
    ? "Not recorded"
    : failed
      ? "Tool failed"
      : errors
        ? "Errors returned"
        : !recognized
          ? "Outcome unknown"
          : "Result recorded";
  let status: TranscriptStatus = !hasResult || !recognized ? "unknown" : "done";
  if (failed || errors) status = "failed";
  if (hasResult && !failed && !errors && recognized) {
    if (value.partial === true) {
      outcome = "Partial result";
      status = "unknown";
    } else if (value.status === "requested")
      outcome = "Requested · not confirmed";
    else if (
      family === "app_build" &&
      action === "publish" &&
      value.revision !== undefined
    )
      outcome = "Published · not runtime verified";
    else if (
      family === "app_source" &&
      ["configure", "update"].includes(action)
    )
      outcome = "Configured · health unknown";
    else if (family === "app_screenshot")
      outcome = parsed.svg
        ? "SVG artifact · not PNG"
        : parsed.images.length
          ? "PNG captured · app-reported"
          : "No image recorded";
    else if (Array.isArray(parsed.value))
      outcome = `${parsed.value.length} records`;
    else if (value.accepted === true) outcome = "Accepted";
  }
  if (!action && hasResult && !failed) {
    outcome = "Outcome unknown";
    status = "unknown";
  }
  if (withheld) {
    outcome = "Response withheld · outcome unknown";
    status = "unknown";
  }
  if (parsed.svg && !failed) {
    outcome = "SVG artifact · not PNG";
    status = "unknown";
  }
  if (!recognized && hasResult)
    add("Response", [
      {
        label: "Format",
        value:
          "Unrecognized response; inspect the permitted response in Raw JSON.",
      },
    ]);
  return {
    title: `${humanizeIdentifier(family)}${
      action ? ` · ${humanizeIdentifier(action)}` : ""
    }`,
    target: typeof manifest.name === "string" ? text(manifest.name) : undefined,
    outcome,
    status,
    facts: resultFacts,
    sections,
    images: parsed.images,
    raw: sanitizeAppValue(parsed.value),
  };
}
