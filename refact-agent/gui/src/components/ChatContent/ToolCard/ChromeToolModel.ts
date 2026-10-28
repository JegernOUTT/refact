import type {
  ActionabilityDiagnostics,
  BrowserActionResponse,
  BrowserAssertionResult,
  BrowserCaptureKind,
  BrowserConsoleCounts,
  BrowserExecutionStep,
  BrowserFrameRecord,
  BrowserPageContext,
  BrowserPageSnapshot,
  BrowserSnapshotArtifact,
} from "../../../services/refact/browser";

export interface ChromeArgs {
  commands?: string;
  request?: {
    steps: Record<string, unknown>[];
    [key: string]: unknown;
  };
}

export interface CommandStats {
  url: string | null;
  screenshotCount: number;
  actionCounts: Partial<Record<string, number>>;
  totalActions: number;
}

export const ACTION_LABELS: Partial<Record<string, string>> = {
  navigate_to: "navigate",
  click_at_element: "click",
  fill_field: "fill",
  type_text_at: "type",
  press_key: "key",
  screenshot: "screenshot",
  eval: "eval",
  scroll_to: "scroll",
  html: "inspect",
  styles: "styles",
  wait_for: "wait",
  wait_for_selector: "wait",
  wait_for_navigation: "wait",
  tab_log: "log",
  open_tab: "tab",
  close_tab: "tab",
  list_tabs: "tabs",
  reload: "reload",
};

export function parseCommandStats(commands: string): CommandStats {
  const lines = commands.split("\n").filter((l) => {
    const t = l.trim();
    return t && !t.startsWith("//") && !t.startsWith("#");
  });

  let url: string | null = null;
  let screenshotCount = 0;
  const actionCounts: Partial<Record<string, number>> = {};

  for (const line of lines) {
    const parts = line.trim().split(/\s+/);
    const cmd = parts[0];
    if (!cmd) continue;

    const label = ACTION_LABELS[cmd] ?? cmd;
    actionCounts[label] = (actionCounts[label] ?? 0) + 1;

    if (cmd === "navigate_to" && parts.length >= 3 && !url) {
      url = parts.slice(2).join(" ");
    }
    if (cmd === "screenshot") {
      screenshotCount++;
    }
  }

  return {
    url,
    screenshotCount,
    actionCounts,
    totalActions: lines.length,
  };
}

export function formatUrl(url: string): string {
  return url.replace(/^file:\/\//, "");
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
export function parseChromeArgs(value: string): ChromeArgs {
  try {
    const parsed = JSON.parse(value) as unknown;
    if (!isRecord(parsed)) return {};

    const commands =
      typeof parsed.commands === "string" ? parsed.commands : undefined;
    const request =
      isRecord(parsed.request) && Array.isArray(parsed.request.steps)
        ? {
            ...parsed.request,
            steps: parsed.request.steps.filter(isRecord),
          }
        : undefined;

    return { commands, request };
  } catch {
    return {};
  }
}

export function isBrowserActionResponse(
  value: unknown,
): value is BrowserActionResponse {
  return (
    isRecord(value) &&
    typeof value.ok === "boolean" &&
    Array.isArray(value.steps) &&
    (typeof value.stabilized === "boolean" || value.stabilized === undefined)
  );
}

export function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

export function optionalNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

export function ariaSnapshotYaml(value: unknown): string | null {
  return isRecord(value) && typeof value.yaml === "string" ? value.yaml : null;
}

export function parseConsoleCounts(value: unknown): BrowserConsoleCounts {
  if (!isRecord(value)) return { errors: 0, warnings: 0 };
  return {
    errors: optionalNumber(value.errors) ?? 0,
    warnings: optionalNumber(value.warnings) ?? 0,
  };
}

export function parseSnapshotArtifact(
  value: unknown,
): BrowserSnapshotArtifact | null {
  if (
    !isRecord(value) ||
    typeof value.kind !== "string" ||
    typeof value.mime !== "string" ||
    typeof value.path !== "string"
  ) {
    return null;
  }
  return {
    kind: value.kind,
    mime: value.mime,
    path: value.path,
    bytes: optionalNumber(value.bytes) ?? 0,
  };
}

export function parsePageSnapshot(value: unknown): BrowserPageSnapshot | null {
  if (!isRecord(value) || typeof value.yaml !== "string") return null;
  return {
    yaml: value.yaml,
    lines: optionalNumber(value.lines) ?? 0,
    bytes: optionalNumber(value.bytes) ?? 0,
    truncated: value.truncated === true,
    artifact: parseSnapshotArtifact(value.artifact),
  };
}

export function parsePageContext(value: unknown): BrowserPageContext | null {
  if (!isRecord(value)) return null;
  return {
    status: optionalNumber(value.status) ?? null,
    console: parseConsoleCounts(value.console),
    snapshot: parsePageSnapshot(value.snapshot),
  };
}

export const CAPTURE_LABELS: Record<BrowserCaptureKind, string> = {
  filmstrip: "Filmstrip",
  element_gallery: "Element gallery",
  element_states: "Element states",
};

export function captureKind(
  data: Record<string, unknown>,
): BrowserCaptureKind | null {
  if (isRecord(data.artifact) && data.artifact.kind === "filmstrip") {
    return "filmstrip";
  }
  if (Array.isArray(data.states) && Array.isArray(data.images)) {
    return "element_states";
  }
  if (typeof data.compose === "string" && Array.isArray(data.images)) {
    return "element_gallery";
  }
  return null;
}

export function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === "string");
}

export interface CaptureFrame extends BrowserFrameRecord {
  artifactPath: string | null;
}

export function artifactPath(value: unknown): string | null {
  if (!isRecord(value) || !isRecord(value.artifact)) return null;
  return typeof value.artifact.path === "string" ? value.artifact.path : null;
}

export function parseFrameRecords(value: unknown): CaptureFrame[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((entry) => {
    if (!isRecord(entry)) return [];
    const index = optionalNumber(entry.index);
    const offset = optionalNumber(entry.offset_ms);
    if (index === undefined || offset === undefined) return [];
    return [
      {
        index,
        offset_ms: offset,
        changed_percent: optionalNumber(entry.changed_percent) ?? null,
        artifactPath: artifactPath(entry),
      },
    ];
  });
}

export function captureDetail(
  kind: BrowserCaptureKind,
  data: Record<string, unknown>,
): string | null {
  const parts: string[] = [];
  if (kind === "filmstrip") {
    const frames = optionalNumber(data.frame_count);
    const columns = optionalNumber(data.columns);
    const rows = optionalNumber(data.rows);
    const duration = optionalNumber(data.duration_ms);
    if (frames !== undefined) {
      parts.push(`${frames} frame${frames === 1 ? "" : "s"}`);
    }
    if (columns !== undefined && rows !== undefined) {
      parts.push(`${columns}×${rows} grid`);
    }
    if (duration !== undefined) parts.push(`${duration}ms`);
  } else if (kind === "element_gallery") {
    const count = optionalNumber(data.count);
    if (count !== undefined) {
      parts.push(`${count} element${count === 1 ? "" : "s"}`);
    }
    if (typeof data.compose === "string") parts.push(data.compose);
  } else {
    parts.push(...stringList(data.states));
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

export function isAttachableImageStep(
  report: BrowserActionResponse,
  data: Record<string, unknown>,
): boolean {
  const artifactKind = isRecord(data.artifact) ? data.artifact.kind : undefined;
  if (report.screenshot && artifactKind !== "filmstrip") return false;
  return (
    typeof data.mime === "string" &&
    data.mime.startsWith("image/") &&
    typeof data.data === "string"
  );
}

export interface StepCaptureEntry {
  stepIndex: number;
  kind: BrowserCaptureKind;
  label: string;
  detail: string | null;
  frames: CaptureFrame[];
  warnings: string[];
  artifactPath: string | null;
  src?: string;
}

export function formatFrameChip(frame: BrowserFrameRecord): string {
  const changed =
    frame.changed_percent === null || frame.changed_percent === undefined
      ? null
      : `${frame.changed_percent.toFixed(1)}% changed`;
  return changed
    ? `+${frame.offset_ms}ms · ${changed}`
    : `+${frame.offset_ms}ms`;
}

export function parseActionabilityDiagnostics(
  value: unknown,
): ActionabilityDiagnostics | null {
  if (
    !isRecord(value) ||
    !Array.isArray(value.call_log) ||
    !value.call_log.every((entry) => typeof entry === "string") ||
    typeof value.timed_out !== "boolean"
  ) {
    return null;
  }

  return {
    call_log: value.call_log,
    timed_out: value.timed_out,
    elapsed_ms: optionalNumber(value.elapsed_ms),
    attempts: optionalNumber(value.attempts),
    attached: optionalBoolean(value.attached),
    visible: optionalBoolean(value.visible),
    stable: optionalBoolean(value.stable),
    enabled: optionalBoolean(value.enabled),
    editable: optionalBoolean(value.editable),
    receives_events: optionalBoolean(value.receives_events),
    intercepting_element:
      typeof value.intercepting_element === "string"
        ? value.intercepting_element
        : undefined,
  };
}

export function parseAssertionResult(
  value: unknown,
): BrowserAssertionResult | null {
  if (
    !isRecord(value) ||
    typeof value.matcher !== "string" ||
    typeof value.passed !== "boolean" ||
    typeof value.soft !== "boolean" ||
    typeof value.attempts !== "number" ||
    typeof value.elapsed_ms !== "number"
  ) {
    return null;
  }

  return {
    matcher: value.matcher,
    passed: value.passed,
    soft: value.soft,
    expected: value.expected,
    received: value.received,
    diff: typeof value.diff === "string" ? value.diff : undefined,
    attempts: value.attempts,
    elapsed_ms: value.elapsed_ms,
  };
}

export function renderAssertionValue(value: unknown): string {
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

export function summarizeStep(step: BrowserExecutionStep): string {
  if (step.ok) return step.summary;
  return step.error ? `${step.summary}: ${step.error}` : step.summary;
}

export interface FillDiagnosticChip {
  label: string;
  tone: "muted" | "success" | "warning";
}

export function fillDiagnostics(
  step: BrowserExecutionStep,
): FillDiagnosticChip[] {
  const chips: FillDiagnosticChip[] = [];
  if (typeof step.field_kind === "string" && step.field_kind.length > 0) {
    chips.push({ label: step.field_kind, tone: "muted" });
  }
  if (typeof step.fill_strategy === "string" && step.fill_strategy.length > 0) {
    chips.push({ label: step.fill_strategy, tone: "muted" });
  }
  if (typeof step.verified === "boolean") {
    chips.push(
      step.verified
        ? { label: "verified", tone: "success" }
        : { label: "unverified", tone: "warning" },
    );
  }
  return chips;
}

export function prettifyActionName(action: string): string {
  return action
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function describeTypedStep(step: Record<string, unknown>): string {
  const action = typeof step.action === "string" ? step.action : "step";

  if (action === "navigate" && typeof step.url === "string") {
    return `Navigate ${formatUrl(step.url)}`;
  }
  if (action === "fill" && isRecord(step.locator)) {
    const locator = step.locator;
    const by = typeof locator.by === "string" ? locator.by : "locator";
    const value =
      typeof locator.value === "string"
        ? locator.value
        : typeof locator.role === "string"
          ? locator.role
          : "element";
    return `Fill ${by}=${value}`;
  }
  if (action === "set_input_files" && isRecord(step.locator)) {
    const count = Array.isArray(step.paths) ? step.paths.length : 0;
    return `Upload ${count} file${count === 1 ? "" : "s"}`;
  }
  if (action === "expect_file_chooser") {
    const count = Array.isArray(step.paths) ? step.paths.length : 0;
    return `Arm file chooser for ${count} file${count === 1 ? "" : "s"}`;
  }
  if (action === "wait_for_download") return "Wait for download";
  if (action === "route_web_socket") return "Route WebSocket";
  if (action === "unroute_web_socket") return "Unroute WebSocket";
  if (action === "send_web_socket_message") return "Send WebSocket message";
  if (action === "wait_for_web_socket_frame") return "Wait for WebSocket frame";
  if (action === "start_har_recording") return "Start HAR recording";
  if (action === "stop_har_recording") return "Stop HAR recording";
  if (action === "route_from_har") return "Route from HAR";
  if (action === "start_coverage") return "Start JS/CSS coverage";
  if (action === "stop_coverage") return "Stop JS/CSS coverage";
  if (action === "add_virtual_authenticator")
    return "Add virtual authenticator";
  if (action === "remove_virtual_authenticator")
    return "Remove virtual authenticator";
  if (action === "list_credentials") return "List authenticator credentials";
  if (action === "add_credential") return "Add authenticator credential";
  if (action === "clear_credentials") return "Clear authenticator credentials";
  if (action === "set_user_verified") return "Set authenticator verification";
  if (action === "drag_and_drop") return "Drag element to target";
  if (action === "drop_files") {
    const count = Array.isArray(step.paths) ? step.paths.length : 0;
    return `Drop ${count} file${count === 1 ? "" : "s"}`;
  }
  if (
    (action === "mouse_move" || action === "mouse_click_xy") &&
    typeof step.x === "number" &&
    typeof step.y === "number"
  ) {
    return `${prettifyActionName(action)} (${step.x}, ${step.y})`;
  }
  if (action === "mouse_drag_xy") return "Mouse drag by coordinates";
  if (action === "mouse_wheel") return "Mouse wheel";
  if (
    (action === "click" || action === "scroll_to") &&
    isRecord(step.locator)
  ) {
    const locator = step.locator;
    const by = typeof locator.by === "string" ? locator.by : "locator";
    const value =
      typeof locator.value === "string"
        ? locator.value
        : typeof locator.role === "string"
          ? locator.role
          : "element";
    return `${prettifyActionName(action)} ${by}=${value}`;
  }
  return prettifyActionName(action);
}

const TARGET_STEP_LIMIT = 3;

export function stepActionName(step: Record<string, unknown>): string {
  return typeof step.action === "string" ? step.action : "step";
}

export function describeStepsTarget(steps: Record<string, unknown>[]): string {
  if (steps.length === 0) return "Browser commands";
  if (steps.length === 1) return describeTypedStep(steps[0]);
  const names = steps
    .slice(0, TARGET_STEP_LIMIT)
    .map((step, index) => {
      const name = prettifyActionName(stepActionName(step));
      return index === 0 ? name : name.toLowerCase();
    })
    .join(", ");
  const more = steps.length - TARGET_STEP_LIMIT;
  return more > 0 ? `${names} · +${more}` : names;
}

export function describeLegacyTarget(stats: CommandStats): string {
  const actions = Object.entries(stats.actionCounts)
    .filter(([key]) => key !== "navigate")
    .map(([key, count]) => (count && count > 1 ? `${count} ${key}` : key));
  if (stats.url) {
    const head = `Navigate ${formatUrl(stats.url)}`;
    return actions.length > 0 ? `${head} · ${actions.join(", ")}` : head;
  }
  if (actions.length === 0) return "Browser commands";
  const [first, ...rest] = actions;
  return [first.charAt(0).toUpperCase() + first.slice(1), ...rest].join(", ");
}

export function urlHost(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const parsed = new URL(url);
    return parsed.host || formatUrl(url);
  } catch {
    return formatUrl(url);
  }
}

export function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

const SECRET_KEYS = new Set([
  "password",
  "private_key",
  "credential_id",
  "user_handle",
  "large_blob",
  "user_name",
  "user_display_name",
]);

export function redactedRequest(request: unknown): string {
  return JSON.stringify(
    request,
    function (
      this: Record<string, unknown>,
      key: string,
      value: unknown,
    ): unknown {
      if (SECRET_KEYS.has(key)) return "[REDACTED]";
      if (key === "value" && typeof this.name === "string") return "[REDACTED]";
      return value;
    },
    2,
  );
}

const ACTIONABILITY_STATES: [keyof ActionabilityDiagnostics, string][] = [
  ["attached", "attached"],
  ["visible", "visible"],
  ["stable", "stable"],
  ["enabled", "enabled"],
  ["editable", "editable"],
  ["receives_events", "receives events"],
];

export function actionabilityText(
  step: BrowserExecutionStep,
  diagnostics: ActionabilityDiagnostics,
): string {
  const facts = [
    diagnostics.timed_out ? "timed out" : "completed",
    diagnostics.elapsed_ms != null ? `${diagnostics.elapsed_ms} ms` : null,
    diagnostics.attempts != null ? `${diagnostics.attempts} attempts` : null,
    `${step.retries} ${step.retries === 1 ? "retry" : "retries"}`,
  ].filter(Boolean);
  const states = ACTIONABILITY_STATES.flatMap(([key, label]) => {
    const value = diagnostics[key];
    return typeof value === "boolean"
      ? [`${label}: ${value ? "yes" : "no"}`]
      : [];
  });
  return [
    facts.join(" · "),
    states.join(" · "),
    diagnostics.intercepting_element
      ? `intercepting element: ${diagnostics.intercepting_element}`
      : null,
    ...diagnostics.call_log,
  ]
    .filter((line): line is string => Boolean(line))
    .join("\n");
}

export function failedStepLines(report: BrowserActionResponse): string[] {
  return report.steps
    .filter((step) => !step.ok)
    .map(
      (step) =>
        `Step ${step.step_index + 1}: ${step.summary}${
          step.error ? ` — ${step.error}` : ""
        }`,
    );
}
