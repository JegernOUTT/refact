import type {
  BrowserActionResponse,
  BrowserElementState,
  BrowserExecutionStep,
  BrowserSnapshotBox,
} from "../../../services/refact/browser";

export interface NetworkSummaryRow {
  method: string;
  url: string;
  status: number | null;
  bytes: number | null;
  elapsedMs: number | null;
  failure: string | null;
  raw: string;
}

export interface RouteRow {
  order: number;
  pattern: string;
  handler: string;
  detail: string | null;
  timesRemaining: number | null;
  harEntries: number | null;
}

export interface RouteChainEntry {
  stepIndex: number;
  routes: RouteRow[];
}

export interface HttpRequestEntry {
  stepIndex: number;
  method: string;
  url: string;
  status: number;
  statusText: string | null;
  redirects: number | null;
  bodyBytes: number | null;
  cookieNames: string[];
  body: string | null;
  bodyLanguage: string;
  artifactPath: string | null;
  artifactBytes: number | null;
}

export interface ClockEntry {
  stepIndex: number;
  action: string;
  offset: string | null;
  installed: boolean;
  paused: boolean;
}

export type ReadoutKind =
  | "bounding_box"
  | "count"
  | "value"
  | "attribute"
  | "state";

export interface ReadoutEntry {
  stepIndex: number;
  kind: ReadoutKind;
  summary: string;
  box: BrowserSnapshotBox | null;
  count: number | null;
  label: string | null;
  value: string | null;
  state: BrowserElementState | null;
}

export interface DeviceCatalog {
  stepIndex: number;
  devices: string[];
  aliases: string[];
}

export interface EmulationChip {
  stepIndex: number;
  label: string;
}

export interface WebSocketRouteRow {
  stepIndex: number;
  pattern: string;
  mode: string;
  routeCount: number | null;
}

export interface WebSocketEventRow {
  sequence: number;
  url: string;
  kind: string;
  direction: "sent" | "received" | "none";
  detail: string | null;
  routed: boolean;
  failed: boolean;
}

export interface WebSocketCloseRow {
  stepIndex: number;
  closed: number;
  code: number | null;
  reason: string | null;
}

export interface CdpEntry {
  stepIndex: number;
  method: string;
  target: string;
  bytes: number;
  warnings: string[];
  result: string | null;
  artifactPath: string | null;
}

export interface ResetEntry {
  stepIndex: number;
  cleared: { label: string; count: number }[];
  flags: string[];
}

export interface BrowserFamilies {
  networkSummary: NetworkSummaryRow[];
  routeChains: RouteChainEntry[];
  httpRequests: HttpRequestEntry[];
  clock: ClockEntry[];
  readouts: ReadoutEntry[];
  devices: DeviceCatalog[];
  emulation: EmulationChip[];
  webSocketRoutes: WebSocketRouteRow[];
  webSocketEvents: WebSocketEventRow[];
  webSocketCloses: WebSocketCloseRow[];
  cdp: CdpEntry[];
  resets: ResetEntry[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((entry): entry is string => typeof entry === "string");
}

function stepData(step: BrowserExecutionStep): Record<string, unknown> | null {
  return isRecord(step.data) ? step.data : null;
}

const NETWORK_SUMMARY_PATTERN =
  /^(\S+) (\S+) (\S+) (\S+) (\S+)(?: ([\s\S]+))?$/;

function optionalMetric(token: string, suffix: string): number | null {
  if (!token.endsWith(suffix)) return null;
  const parsed = Number(token.slice(0, -suffix.length));
  return Number.isFinite(parsed) ? parsed : null;
}

export function parseNetworkSummaryLine(line: string): NetworkSummaryRow {
  const match = NETWORK_SUMMARY_PATTERN.exec(line.trim());
  if (!match) {
    return {
      method: "",
      url: line.trim(),
      status: null,
      bytes: null,
      elapsedMs: null,
      failure: null,
      raw: line,
    };
  }
  const [, method, url, status, bytes, elapsed, failure] = match;
  const parsedStatus = Number(status);
  return {
    method,
    url,
    status: Number.isInteger(parsedStatus) ? parsedStatus : null,
    bytes: optionalMetric(bytes, "b"),
    elapsedMs: optionalMetric(elapsed, "ms"),
    failure: failure ? failure.trim() : null,
    raw: line,
  };
}

function describePattern(value: unknown): string {
  if (typeof value === "string") return value;
  if (isRecord(value) && typeof value.source === "string") {
    const flags = typeof value.flags === "string" ? value.flags : "";
    return `/${value.source}/${flags}`;
  }
  return "unknown pattern";
}

function describeHandler(handler: Record<string, unknown>): {
  type: string;
  detail: string | null;
} {
  const type = text(handler.type) ?? "route";
  if (type === "fulfill") {
    const status = finiteNumber(handler.status);
    const contentType = text(handler.content_type);
    const parts = [
      status === null ? null : `status ${status}`,
      contentType,
    ].filter((part): part is string => part !== null);
    return { type, detail: parts.length > 0 ? parts.join(" · ") : null };
  }
  if (type === "abort") return { type, detail: text(handler.reason) };
  if (type === "continue") {
    const parts = [text(handler.method), text(handler.url)].filter(
      (part): part is string => part !== null,
    );
    return { type, detail: parts.length > 0 ? parts.join(" ") : null };
  }
  return { type, detail: null };
}

function parseRouteRow(value: unknown, fallbackOrder: number): RouteRow | null {
  if (!isRecord(value)) return null;
  const handler = isRecord(value.handler)
    ? describeHandler(value.handler)
    : null;
  const har = isRecord(value.har) ? finiteNumber(value.har.entry_count) : null;
  return {
    order: finiteNumber(value.order) ?? fallbackOrder,
    pattern: describePattern(value.pattern),
    handler: handler?.type ?? "route",
    detail: handler?.detail ?? null,
    timesRemaining: finiteNumber(value.times_remaining),
    harEntries: har,
  };
}

function parseRouteChain(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): RouteChainEntry | null {
  if (!Array.isArray(data.routes)) return null;
  const routes = data.routes.flatMap((entry, index) => {
    const row = parseRouteRow(entry, index);
    return row ? [row] : [];
  });
  return { stepIndex: step.step_index, routes };
}

function bodyLanguageFor(body: string): string {
  const trimmed = body.trimStart();
  return trimmed.startsWith("{") || trimmed.startsWith("[") ? "json" : "text";
}

function prettifyBody(body: string): string {
  try {
    const parsed = JSON.parse(body) as unknown;
    if (typeof parsed !== "object" || parsed === null) return body;
    return JSON.stringify(parsed, null, 2);
  } catch {
    return body;
  }
}

function parseHttpRequest(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): HttpRequestEntry | null {
  const payload = data.http_request;
  if (!isRecord(payload)) return null;
  const status = finiteNumber(payload.status);
  if (status === null) return null;
  const cookies = isRecord(payload.set_cookies)
    ? stringList(payload.set_cookies.names)
    : [];
  const artifact = isRecord(payload.artifact) ? payload.artifact : null;
  const body = text(payload.body);
  return {
    stepIndex: step.step_index,
    method: text(payload.method) ?? "GET",
    url: text(payload.url) ?? "Unknown URL",
    status,
    statusText: text(payload.status_text),
    redirects: finiteNumber(payload.redirects),
    bodyBytes: finiteNumber(payload.body_bytes),
    cookieNames: cookies,
    body: body === null ? null : prettifyBody(body),
    bodyLanguage: body === null ? "text" : bodyLanguageFor(body),
    artifactPath: artifact ? text(artifact.path) : null,
    artifactBytes: artifact ? finiteNumber(artifact.bytes) : null,
  };
}

export function formatClockOffset(ticksMs: number): string {
  if (ticksMs % 1_000 !== 0) return `+${ticksMs}ms`;
  const totalSeconds = Math.trunc(ticksMs / 1_000);
  const hours = Math.trunc(totalSeconds / 3_600);
  const minutes = Math.trunc((totalSeconds % 3_600) / 60);
  const seconds = totalSeconds % 60;
  const pad = (value: number) => String(value).padStart(2, "0");
  if (hours > 0) return `+${pad(hours)}:${pad(minutes)}`;
  return `+${pad(minutes)}:${pad(seconds)}`;
}

function clockOffsetFromRequest(
  requestStep: Record<string, unknown> | null,
): string | null {
  if (!requestStep) return null;
  const ticks = finiteNumber(requestStep.ticks_ms);
  if (ticks !== null) return formatClockOffset(ticks);
  const time = finiteNumber(requestStep.time_ms);
  if (time === null) return null;
  return new Date(time).toISOString().slice(11, 19);
}

function parseClock(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
  requestStep: Record<string, unknown> | null,
): ClockEntry | null {
  const payload = data.clock;
  if (!isRecord(payload)) return null;
  return {
    stepIndex: step.step_index,
    action: (requestStep ? text(requestStep.action) : null) ?? "clock",
    offset: clockOffsetFromRequest(requestStep),
    installed: payload.installed === true,
    paused: payload.paused === true,
  };
}

function parseBoundingBox(value: unknown): BrowserSnapshotBox | null {
  if (!isRecord(value)) return null;
  const x = finiteNumber(value.x);
  const y = finiteNumber(value.y);
  const width = finiteNumber(value.width);
  const height = finiteNumber(value.height);
  if (x === null || y === null || width === null || height === null) {
    return null;
  }
  return { x, y, width, height };
}

const ELEMENT_STATE_KEYS = [
  "visible",
  "enabled",
  "editable",
  "stable",
  "checked",
] as const;

function parseElementState(value: unknown): BrowserElementState | null {
  if (!isRecord(value)) return null;
  const hasFlag = ELEMENT_STATE_KEYS.some((key) => key in value);
  if (!hasFlag) return null;
  const checked = value.checked;
  return {
    visible: typeof value.visible === "boolean" ? value.visible : undefined,
    enabled: typeof value.enabled === "boolean" ? value.enabled : undefined,
    editable: typeof value.editable === "boolean" ? value.editable : undefined,
    stable: typeof value.stable === "boolean" ? value.stable : undefined,
    checked:
      typeof checked === "boolean" || typeof checked === "string"
        ? checked
        : null,
  };
}

function renderValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === undefined) return "null";
  return JSON.stringify(value);
}

function parseReadout(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): ReadoutEntry | null {
  const keys = Object.keys(data);
  const base = {
    stepIndex: step.step_index,
    summary: step.summary,
    box: null,
    count: null,
    label: null,
    value: null,
    state: null,
  };
  if (keys.length === 1 && keys[0] === "bounding_box") {
    return {
      ...base,
      kind: "bounding_box",
      box: parseBoundingBox(data.bounding_box),
    };
  }
  if (keys.length === 1 && keys[0] === "count") {
    const count = finiteNumber(data.count);
    return count === null ? null : { ...base, kind: "count", count };
  }
  if (keys.length === 1 && keys[0] === "value") {
    return { ...base, kind: "value", value: renderValue(data.value) };
  }
  if (
    keys.length === 2 &&
    keys.includes("attribute") &&
    keys.includes("value")
  ) {
    return {
      ...base,
      kind: "attribute",
      label: text(data.attribute),
      value: renderValue(data.value),
    };
  }
  const state = parseElementState(data.state);
  return state === null ? null : { ...base, kind: "state", state };
}

function parseDevices(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): DeviceCatalog | null {
  if (!Array.isArray(data.devices)) return null;
  return {
    stepIndex: step.step_index,
    devices: stringList(data.devices),
    aliases: stringList(data.aliases),
  };
}

function parseEmulation(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): EmulationChip | null {
  if ("network_conditions" in data) {
    const conditions = isRecord(data.network_conditions)
      ? data.network_conditions
      : null;
    if (data.offline === true) {
      return { stepIndex: step.step_index, label: "Offline" };
    }
    if (!conditions) {
      return { stepIndex: step.step_index, label: "Network throttling off" };
    }
    const latency = finiteNumber(conditions.latency_ms);
    const download = finiteNumber(conditions.download_kbps);
    const upload = finiteNumber(conditions.upload_kbps);
    const parts = [
      latency === null ? null : `${latency}ms`,
      download === null ? null : `↓${download} kbps`,
      upload === null ? null : `↑${upload} kbps`,
    ].filter((part): part is string => part !== null);
    return { stepIndex: step.step_index, label: parts.join(" · ") };
  }
  const rate = finiteNumber(data.cpu_throttling_rate);
  if (rate !== null) {
    return {
      stepIndex: step.step_index,
      label: rate > 1 ? `CPU ${rate}× slower` : "CPU throttling off",
    };
  }
  const device = isRecord(data.device) ? data.device : null;
  if (!device) return null;
  const width = finiteNumber(device.width);
  const height = finiteNumber(device.height);
  const size = width === null || height === null ? null : `${width}×${height}`;
  const parts = [
    text(device.name),
    size,
    device.has_touch === true ? "touch" : null,
  ].filter((part): part is string => part !== null);
  return { stepIndex: step.step_index, label: parts.join(" · ") };
}

function parseCdp(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): CdpEntry | null {
  const payload = data.cdp_send;
  if (!isRecord(payload)) return null;
  const artifact = isRecord(payload.artifact) ? payload.artifact : null;
  return {
    stepIndex: step.step_index,
    method: text(payload.method) ?? "CDP",
    target: text(payload.target) ?? "page",
    bytes: finiteNumber(payload.bytes) ?? 0,
    warnings: stringList(payload.warnings),
    result:
      payload.result === undefined || payload.result === null
        ? null
        : JSON.stringify(payload.result, null, 2),
    artifactPath: artifact ? text(artifact.path) : null,
  };
}

const RESET_COUNTS: [string, string][] = [
  ["routes", "routes"],
  ["har_replays", "HAR replays"],
  ["websocket_routes", "WebSocket routes"],
  ["locator_handlers", "locator handlers"],
  ["authenticators", "authenticators"],
  ["init_scripts", "init scripts"],
];

const RESET_FLAGS: [string, string][] = [
  ["throttling_cleared", "throttling cleared"],
  ["emulation_cleared", "emulation cleared"],
  ["clock_cleared", "clock cleared"],
  ["service_worker_block_cleared", "service worker block cleared"],
];

function parseReset(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): ResetEntry | null {
  const payload = data.reset;
  if (!isRecord(payload)) return null;
  const cleared = RESET_COUNTS.flatMap(([key, label]) => {
    const count = finiteNumber(payload[key]);
    return count === null ? [] : [{ label, count }];
  });
  const flags = RESET_FLAGS.flatMap(([key, label]) =>
    payload[key] === true ? [label] : [],
  );
  return { stepIndex: step.step_index, cleared, flags };
}

function collectWebSocketRoutes(
  requestSteps: Record<string, unknown>[] | null,
  dataByStep: Map<number, Record<string, unknown>>,
): WebSocketRouteRow[] {
  if (!requestSteps) return [];
  return requestSteps.flatMap((requestStep, index) => {
    if (requestStep.action !== "route_web_socket") return [];
    const data = dataByStep.get(index);
    return [
      {
        stepIndex: index,
        pattern: describePattern(requestStep.pattern),
        mode: text(requestStep.mode) ?? "mock",
        routeCount: data ? finiteNumber(data.route_count) : null,
      },
    ];
  });
}

function parseWebSocketClose(
  step: BrowserExecutionStep,
  data: Record<string, unknown>,
): WebSocketCloseRow | null {
  const closed = finiteNumber(data.closed);
  if (closed === null) return null;
  return {
    stepIndex: step.step_index,
    closed,
    code: finiteNumber(data.code),
    reason: text(data.reason),
  };
}

function webSocketDirection(kind: string): "sent" | "received" | "none" {
  if (kind === "frame_sent") return "sent";
  if (kind === "frame_received" || kind === "handshake_response") {
    return "received";
  }
  return "none";
}

function parseWebSocketEvents(
  report: BrowserActionResponse,
): WebSocketEventRow[] {
  if (!report.websockets) return [];
  return report.websockets.map((event) => ({
    sequence: event.sequence,
    url: event.url,
    kind: event.kind,
    direction: webSocketDirection(event.kind),
    detail:
      event.data ??
      event.error ??
      (event.status === undefined ? null : String(event.status)),
    routed: event.routed,
    failed: event.kind === "error",
  }));
}

export function collectBrowserFamilies(
  report: BrowserActionResponse | null,
  requestSteps: Record<string, unknown>[] | null,
): BrowserFamilies {
  const families: BrowserFamilies = {
    networkSummary: [],
    routeChains: [],
    httpRequests: [],
    clock: [],
    readouts: [],
    devices: [],
    emulation: [],
    webSocketRoutes: [],
    webSocketEvents: [],
    webSocketCloses: [],
    cdp: [],
    resets: [],
  };
  if (!report) return families;

  families.networkSummary = (report.network_summary ?? []).map(
    parseNetworkSummaryLine,
  );
  families.webSocketEvents = parseWebSocketEvents(report);

  const dataByStep = new Map<number, Record<string, unknown>>();

  for (const step of report.steps) {
    const data = stepData(step);
    if (!data) continue;
    dataByStep.set(step.step_index, data);
    const requestStep = requestSteps?.[step.step_index] ?? null;

    const routeChain = parseRouteChain(step, data);
    if (routeChain) families.routeChains.push(routeChain);

    const httpRequest = parseHttpRequest(step, data);
    if (httpRequest) families.httpRequests.push(httpRequest);

    const clock = parseClock(step, data, requestStep);
    if (clock) families.clock.push(clock);

    const readout = parseReadout(step, data);
    if (readout) families.readouts.push(readout);

    const devices = parseDevices(step, data);
    if (devices) families.devices.push(devices);

    const emulation = parseEmulation(step, data);
    if (emulation) families.emulation.push(emulation);

    const webSocketClose = parseWebSocketClose(step, data);
    if (webSocketClose) families.webSocketCloses.push(webSocketClose);

    const cdp = parseCdp(step, data);
    if (cdp) families.cdp.push(cdp);

    const reset = parseReset(step, data);
    if (reset) families.resets.push(reset);
  }

  families.webSocketRoutes = collectWebSocketRoutes(requestSteps, dataByStep);

  return families;
}
