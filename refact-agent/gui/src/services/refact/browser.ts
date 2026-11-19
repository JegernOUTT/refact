import { createAction } from "@reduxjs/toolkit";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { FetchArgs, FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { RootState } from "../../app/store";
import {
  BROWSER_ACTION,
  BROWSER_START,
  BROWSER_STOP,
  BROWSER_SCREENSHOT,
  BROWSER_CONTEXT,
  BROWSER_CURL,
  BROWSER_ELEMENT_PICK,
  BROWSER_ELEMENT_PICK_RESULT,
  BROWSER_ELEMENT_PICK_CANCEL,
  BROWSER_RECORD_ANIMATION,
  BROWSER_HANDOFF,
  BROWSER_STATUS,
  BROWSER_CONTEXT_ESTIMATE,
  BROWSER_ANNOTATE_START,
  BROWSER_ANNOTATE_RESULT,
  BROWSER_ANNOTATE_CLEAR,
} from "./consts";
import { buildApiUrlFromState } from "./apiUrl";

export type BrowserStartRequest = {
  chat_id: string;
};

export type BrowserStartResponse = {
  runtime_id: string;
  status: "started" | "already_running";
};

export type BrowserStopRequest = {
  chat_id: string;
};

export type BrowserStopResponse = {
  status: "stopped";
};

export type BrowserScreenshotRequest = {
  chat_id: string;
  full_page: boolean;
};

export type BrowserScreenshotResponse = {
  mime: string;
  data: string;
  url: string;
  title: string;
};

export type BrowserContextRequest = {
  chat_id: string;
  max_bytes?: number;
  last_n_actions?: number;
  skip_cursor?: boolean;
};

export type BrowserContextResponse = {
  url: string;
  title: string;
  actions: unknown[];
  console: unknown[];
  network: BrowserNetworkEntry[];
  mutations: unknown[];
  total_bytes: number;
};

export type BrowserCurlRequest = {
  chat_id: string;
};

export type BrowserCurlResponse = {
  curl: string;
  url: string;
  method: string;
  status: number;
};

export type BrowserElementPickRequest = {
  chat_id: string;
};

export type BrowserElementPickResponse = {
  status: "picker_active";
};

export type BrowserElementPickResultRequest = {
  chat_id: string;
};

export type BrowserElementPickResultResponse =
  | { status: "waiting" }
  | {
      selector: string;
      innerText: string;
      bbox: { x: number; y: number; width: number; height: number };
    };

export type BrowserElementPickCancelRequest = {
  chat_id: string;
};

export type BrowserElementPickCancelResponse = {
  status: "cancelled";
};

export type BrowserRecordAnimationRequest = {
  chat_id: string;
};

export type BrowserRecordAnimationResponse = {
  frames: { mime: string; data: string; timestamp: number }[];
};

export type BrowserHandoffRequest = {
  from_chat_id: string;
  to_chat_id: string;
};

export type BrowserHandoffResponse = {
  runtime_id: string;
  status: string;
  from_chat_id: string;
  to_chat_id: string;
};

export type BrowserAnnotateStartRequest = {
  chat_id: string;
};

export type BrowserAnnotateStartResponse = {
  status: "started" | "already_active";
};

export type BrowserAnnotation = {
  index: number;
  type?: "element" | "rect";
  selector: string;
  innerText: string;
  caption?: string;
  bbox: { x: number; y: number; width: number; height: number };
};

export type BrowserAnnotateResultRequest = {
  chat_id: string;
};

export type BrowserAnnotateResultResponse = {
  annotations: BrowserAnnotation[];
  active: boolean;
};

export type BrowserAnnotateClearRequest = {
  chat_id: string;
};

export type BrowserAnnotateClearResponse = {
  status: "cleared";
};

export type BrowserContextEstimateRequest = {
  chat_id: string;
  include_actions: boolean;
  include_console: boolean;
  include_network: boolean;
  include_mutations: boolean;
  include_screenshot: boolean;
  last_n_actions: number;
  last_n_console: number;
  last_n_network: number;
};

export type BrowserContextEstimateResponse = {
  estimated_bytes: number;
};

export type BrowserStatusRequest = {
  chat_id: string;
};

export type BrowserStatusResponse = {
  runtime_id: string | null;
  connected: boolean;
  active_tab?: string | null;
  url?: string;
  title?: string;
  tab_urls?: string[];
  tabs?: { tab_id: string; url: string; title: string }[];
  idle_seconds?: number;
  idle_timeout?: number;
};

export type BrowserLocator = {
  by: string;
  value?: string;
  exact?: boolean;
  role?: string;
  name?: string;
  nth?: number;
  within?: string;
};

export type BrowserTabTarget = { type: "active" } | { type: "id"; id: string };

export type BrowserMouseButton = "left" | "middle" | "right";

export type BrowserPosition = { x: number; y: number };

export type BrowserPointerStep =
  | {
      action: "drag_and_drop";
      source: BrowserLocator;
      target: BrowserLocator;
      source_position?: BrowserPosition;
      target_position?: BrowserPosition;
    }
  | { action: "drop_files"; target: BrowserLocator; paths: string[] }
  | { action: "mouse_move"; x: number; y: number; steps?: number }
  | { action: "mouse_down" | "mouse_up"; button?: BrowserMouseButton }
  | {
      action: "mouse_click_xy";
      x: number;
      y: number;
      button?: BrowserMouseButton;
      click_count?: number;
      delay?: number;
    }
  | {
      action: "mouse_drag_xy";
      start_x: number;
      start_y: number;
      end_x: number;
      end_y: number;
    }
  | { action: "mouse_wheel"; delta_x: number; delta_y: number };

export type BrowserStep =
  | BrowserPointerStep
  | {
      action: string;
      [key: string]: unknown;
    };

export type BrowserActionRequest = {
  chat_id: string;
  session?: "shared_default";
  target?: BrowserTabTarget;
  attach_screenshot?: boolean;
  steps: BrowserStep[];
};

export type BrowserConsoleEntry = {
  timestamp: number;
  level: string;
  text: string;
};

export type BrowserNetworkTiming = {
  start_time: number;
  request_start?: number | null;
  response_start?: number | null;
  response_end?: number | null;
};

export type BrowserNetworkEntry = {
  timestamp: number;
  method: string;
  url: string;
  resource_type: string;
  status: number | null;
  status_text?: string | null;
  request_headers?: Record<string, string>;
  response_headers?: Record<string, string>;
  frame_id?: string | null;
  loader_id?: string | null;
  document_id?: string | null;
  redirect_from?: string | null;
  timing?: BrowserNetworkTiming | null;
  encoded_data_length?: number | null;
  transfer_size?: number | null;
  failure_text?: string | null;
  from_service_worker: boolean;
  is_navigation_request: boolean;
};

export type BrowserReportScreenshot = {
  mime: string;
  data: string;
};

export type BrowserDialogInfo = {
  type: "alert" | "confirm" | "prompt" | "beforeunload";
  message: string;
  default_value: string;
  action: "accepted" | "dismissed";
  automatic: boolean;
};

export type BrowserUploadInfo = {
  paths: string[];
  source: string;
  in_memory_payloads: boolean;
};

export type BrowserDownloadState = "in_progress" | "completed" | "canceled";

export type BrowserDownloadInfo = {
  guid: string;
  url: string;
  frame_id: string;
  suggested_filename: string;
  local_path: string;
  received_bytes: number;
  total_bytes: number;
  state: BrowserDownloadState;
};

export type LocatorHandlerFiring = {
  name: string;
  action: string;
  outcome: string;
  ok: boolean;
};

export type BrowserSnapshotBox = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserAriaSnapshotNode = {
  role: string;
  name?: string | null;
  ref?: string | null;
  box?: BrowserSnapshotBox | null;
};

export type BrowserSnapshotGeneration = {
  document_generation: number;
  frame_generation: number;
  refs: Record<
    string,
    {
      role: string;
      name?: string | null;
    }
  >;
};

export type BrowserAriaSnapshot = {
  yaml: string;
  nodes: BrowserAriaSnapshotNode[];
  generation?: BrowserSnapshotGeneration | null;
};

export type ActionabilityDiagnostics = {
  call_log: string[];
  timed_out: boolean;
  elapsed_ms?: number;
  attempts?: number;
  attached?: boolean;
  visible?: boolean;
  stable?: boolean;
  enabled?: boolean;
  editable?: boolean;
  receives_events?: boolean;
  intercepting_element?: string;
};

export type BrowserAssertionResult = {
  matcher: string;
  passed: boolean;
  soft: boolean;
  expected: unknown;
  received: unknown;
  diff?: string | null;
  attempts: number;
  elapsed_ms: number;
};

export type BrowserImageArtifact = {
  kind: "image";
  mime: string;
  data: string;
  width: number;
  height: number;
  bytes: number;
};

export type BrowserPdfArtifact = {
  kind: "pdf";
  mime: "application/pdf";
  path: string;
  bytes: number;
  data?: string | null;
};

export type BrowserHarArtifact = {
  kind: "har";
  mime: "application/json";
  path: string;
  bytes: number;
  entry_count: number;
};

export type BrowserCoverageArtifact = {
  kind: "coverage";
  mime: "application/json";
  path: string;
  bytes: number;
  resource_count: number;
};

export type BrowserArtifact =
  | BrowserImageArtifact
  | BrowserPdfArtifact
  | BrowserHarArtifact
  | BrowserCoverageArtifact;

export type BrowserWebSocketEvent = {
  sequence: number;
  socket_id: string;
  url: string;
  kind:
    | "created"
    | "handshake_response"
    | "frame_sent"
    | "frame_received"
    | "closed"
    | "error";
  data?: string;
  opcode?: number;
  status?: number;
  error?: string;
  routed: boolean;
};

export type BrowserExecutionStep = {
  step_index: number;
  ok: boolean;
  summary: string;
  error?: string | null;
  data?:
    | (Record<string, unknown> & { artifact?: BrowserArtifact })
    | BrowserAriaSnapshot
    | null;
  field_kind?: string | null;
  fill_strategy?: string | null;
  verified?: boolean | null;
  retries: number;
  actionability?: ActionabilityDiagnostics;
  assertion?: BrowserAssertionResult;
  locator_echo?: string | null;
};

export type BrowserConsoleCounts = {
  errors: number;
  warnings: number;
};

export type BrowserSnapshotArtifact = {
  kind: string;
  mime: string;
  path: string;
  bytes: number;
};

export type BrowserPageSnapshot = {
  yaml: string;
  lines: number;
  bytes: number;
  truncated: boolean;
  artifact?: BrowserSnapshotArtifact | null;
};

export type BrowserPageContext = {
  status?: number | null;
  console: BrowserConsoleCounts;
  snapshot?: BrowserPageSnapshot | null;
};

export type BrowserFrameRecord = {
  index: number;
  offset_ms: number;
  changed_percent?: number | null;
};

export type BrowserCaptureKind =
  | "filmstrip"
  | "element_gallery"
  | "element_states";

export type BrowserStepCapture = {
  step_index: number;
  kind: BrowserCaptureKind;
  label: string;
  detail: string | null;
  frames: BrowserFrameRecord[];
  warnings: string[];
};

export type BrowserTabOpener = {
  tab_id: string;
  frame_id?: string | null;
};

export type BrowserReportTab = {
  id: string;
  target_id: string;
  url: string;
  title: string;
  active: boolean;
  opener?: BrowserTabOpener | null;
  opened_by_step?: number | null;
};

export type BrowserUrlPattern = string | { source: string; flags?: string };

export type BrowserRouteHandler =
  | {
      type: "fulfill";
      status: number;
      headers?: Record<string, string>;
      body?: string;
      content_type?: string;
      body_base64?: boolean;
    }
  | { type: "abort"; reason: string }
  | {
      type: "continue";
      url?: string;
      method?: string;
      headers?: Record<string, string>;
      post_data?: string;
    };

export type BrowserHarRouteInfo = {
  entry_count: number;
  not_found?: string;
};

export type BrowserRouteInfo = {
  pattern: BrowserUrlPattern;
  handler: BrowserRouteHandler;
  har?: BrowserHarRouteInfo | null;
  times_remaining?: number | null;
  order?: number;
};

export type BrowserRouteInterception = {
  url: string;
  method: string;
  pattern: BrowserUrlPattern;
  action: "fulfill" | "abort" | "continue";
  request_headers?: Record<string, string>;
  request_body_preview?: string;
  response_body_preview?: string;
  status?: number;
  reason?: string;
  redirect_hop: boolean;
};

export type BrowserContextSummary = {
  viewport?: string;
  locale?: string;
  timezone?: string;
  color_scheme?: string;
  permissions?: string[];
  cookie_count: number;
  local_storage_count: number;
  session_storage_count: number;
  offline: boolean;
  http_credentials: boolean;
};

export type BrowserHttpRequestResult = {
  method: string;
  url: string;
  status: number;
  status_text?: string | null;
  redirects?: number | null;
  headers?: Record<string, string> | null;
  body_bytes?: number | null;
  set_cookies?: { count: number; names: string[] } | null;
  body?: string | null;
  artifact?: { path: string; bytes: number } | null;
};

export type BrowserCdpResult = {
  method: string;
  target: string;
  warnings: string[];
  bytes: number;
  result?: unknown;
  artifact?: BrowserSnapshotArtifact | null;
};

export type BrowserClockState = {
  installed: boolean;
  paused: boolean;
};

export type BrowserResetSummary = {
  routes: number;
  har_replays: number;
  websocket_routes: number;
  locator_handlers: number;
  authenticators: number;
  init_scripts: number;
  offline: boolean;
  throttling_cleared: boolean;
  emulation_cleared: boolean;
  clock_cleared: boolean;
  service_worker_block_cleared: boolean;
};

export type BrowserNetworkConditions = {
  latency_ms: number;
  download_kbps: number;
  upload_kbps: number;
};

export type BrowserElementState = {
  visible?: boolean;
  enabled?: boolean;
  editable?: boolean;
  stable?: boolean;
  checked?: boolean | string | null;
};

export type BrowserActionResponse = {
  ok: boolean;
  steps: BrowserExecutionStep[];
  warnings?: string[];
  network_summary?: string[];
  url?: string | null;
  title?: string | null;
  stabilized?: boolean;
  console?: BrowserConsoleEntry[];
  page_errors?: string[];
  network?: BrowserNetworkEntry[];
  websockets?: BrowserWebSocketEvent[];
  locator_handlers?: LocatorHandlerFiring[];
  dialogs?: BrowserDialogInfo[];
  uploads?: BrowserUploadInfo[];
  downloads?: BrowserDownloadInfo[];
  new_tabs?: BrowserReportTab[];
  active_routes?: BrowserRouteInfo[];
  intercepted_requests?: BrowserRouteInterception[];
  context?: BrowserContextSummary | null;
  screenshot?: BrowserReportScreenshot | null;
  page?: BrowserPageContext | null;
};

export const BROWSER_PROJECT_STATUS = "/v1/browser/project/status";
export const BROWSER_PROJECT_START = "/v1/browser/project/start";
export const BROWSER_PROJECT_TAB = "/v1/browser/project/tab";
export const BROWSER_PROJECT_NAVIGATE = "/v1/browser/project/navigate";
export const BROWSER_PROJECT_INPUT = "/v1/browser/project/input";
export const BROWSER_PROJECT_RESIZE = "/v1/browser/project/resize";
export const BROWSER_PROJECT_FRAME = "/v1/browser/project/frame";

export type ProjectBrowserTab = {
  tab_id: string;
  url: string;
  title: string;
  active: boolean;
  name: string | null;
  opened_by_chat: string | null;
  last_chat: string | null;
  last_used_ms: number | null;
};

export type ProjectBrowserViewport = { width: number; height: number };

export type ProjectBrowserStatus = {
  project_id: string | null;
  running: boolean;
  runtime_id: string | null;
  connected: boolean;
  busy: boolean;
  active_tab: string | null;
  tabs: ProjectBrowserTab[];
  new_tab_id: string | null;
};

export type ProjectBrowserTabRequest =
  | { op: "close"; tab_id: string }
  | { op: "new"; url?: string; chat_id?: string };

export type ProjectBrowserNavigateRequest =
  | { url: string; tab_id?: string }
  | { op: "back" | "forward" | "reload"; tab_id?: string };

export type ProjectBrowserModifier = "Alt" | "Ctrl" | "Meta" | "Shift";

export type ProjectBrowserInputFence = { runtime_id: string; tab_id: string };

export type ProjectBrowserInputRequest = ProjectBrowserInputFence &
  (
    | { kind: "click"; x: number; y: number; button: BrowserMouseButton }
    | { kind: "key"; key: string; modifiers: ProjectBrowserModifier[] }
    | { kind: "text"; text: string }
    | { kind: "wheel"; x: number; y: number; dx: number; dy: number }
  );

export type ProjectBrowserResizeRequest = ProjectBrowserViewport & {
  tab_id?: string;
};

export type ProjectBrowserFrameRequest = {
  max_side?: number;
  tab_id?: string;
};

export type ProjectBrowserFrame = {
  runtime_id: string;
  mime: string;
  data: string;
  tab_id: string;
  url: string;
  title: string;
  width: number;
  height: number;
  viewport: ProjectBrowserViewport;
};

const PROJECT_BROWSER_FRAME_MIMES = ["image/jpeg", "image/png", "image/webp"];

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

const isPositiveSize = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value) && value > 0;

const optionalString = (value: unknown): string | null | undefined =>
  value === undefined || value === null
    ? null
    : typeof value === "string"
      ? value
      : undefined;

const optionalNonEmptyString = (value: unknown): string | null | undefined => {
  const text = optionalString(value);
  return text === "" ? null : text;
};

function normalizeProjectBrowserTab(value: unknown): ProjectBrowserTab | null {
  if (!isRecord(value)) return null;
  const { tab_id, url, title, last_used_ms } = value;
  if (typeof tab_id !== "string" || tab_id.length === 0) return null;
  if (typeof url !== "string" || typeof title !== "string") return null;
  const name = optionalNonEmptyString(value.name);
  const openedByChat = optionalNonEmptyString(value.opened_by_chat);
  const lastChat = optionalNonEmptyString(value.last_chat);
  if (
    name === undefined ||
    openedByChat === undefined ||
    lastChat === undefined
  )
    return null;
  return {
    tab_id,
    url,
    title,
    active: value.active === true,
    name,
    opened_by_chat: openedByChat,
    last_chat: lastChat,
    last_used_ms:
      typeof last_used_ms === "number" && Number.isFinite(last_used_ms)
        ? last_used_ms
        : null,
  };
}

export function normalizeProjectBrowserStatus(
  value: unknown,
): ProjectBrowserStatus | null {
  if (!isRecord(value)) return null;
  if (typeof value.running !== "boolean") return null;
  if (typeof value.connected !== "boolean") return null;
  if (!Array.isArray(value.tabs)) return null;
  const runtimeId = optionalString(value.runtime_id);
  const activeTab = optionalString(value.active_tab);
  const projectId = optionalString(value.project_id);
  const newTabId = optionalNonEmptyString(value.new_tab_id);
  if (
    runtimeId === undefined ||
    activeTab === undefined ||
    projectId === undefined ||
    newTabId === undefined
  )
    return null;
  const tabs: ProjectBrowserTab[] = [];
  for (const entry of value.tabs) {
    const tab = normalizeProjectBrowserTab(entry);
    if (!tab) return null;
    tabs.push(tab);
  }
  return {
    project_id: projectId,
    running: value.running,
    runtime_id: runtimeId,
    connected: value.connected,
    busy: value.busy === true,
    active_tab: activeTab,
    tabs,
    new_tab_id: newTabId,
  };
}

export function normalizeProjectBrowserFrame(
  value: unknown,
): ProjectBrowserFrame | null {
  if (!isRecord(value)) return null;
  const { runtime_id, mime, data, tab_id, url, title, width, height } = value;
  const viewport = value.viewport;
  if (typeof runtime_id !== "string" || runtime_id.length === 0) return null;
  if (
    !isRecord(viewport) ||
    !isPositiveSize(viewport.width) ||
    !isPositiveSize(viewport.height)
  )
    return null;
  if (typeof mime !== "string" || !PROJECT_BROWSER_FRAME_MIMES.includes(mime))
    return null;
  if (typeof data !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(data))
    return null;
  if (typeof tab_id !== "string" || tab_id.length === 0) return null;
  if (!isPositiveSize(width) || !isPositiveSize(height)) return null;
  return {
    runtime_id,
    mime,
    data,
    tab_id,
    url: typeof url === "string" ? url : "",
    title: typeof title === "string" ? title : "",
    width,
    height,
    viewport: { width: viewport.width, height: viewport.height },
  };
}

export function projectBrowserErrorMessage(error: unknown): string {
  if (isRecord(error)) {
    const data = error.data;
    if (isRecord(data)) {
      if (typeof data.detail === "string" && data.detail) return data.detail;
      if (typeof data.error === "string" && data.error) return data.error;
    }
    if (typeof data === "string" && data && !data.trimStart().startsWith("<"))
      return data;
    if (typeof error.error === "string" && error.error) return error.error;
    if (typeof error.status === "number")
      return `Browser request failed (${error.status})`;
  }
  return "Browser request failed";
}

export const isProjectBrowserConflict = (error: unknown): boolean =>
  isRecord(error) && error.status === 409;

export const isProjectBrowserTabGone = (error: unknown): boolean =>
  isRecord(error) && error.status === 410;

type ProjectBrowserQueryResult =
  | { data: unknown; error?: undefined }
  | { error: FetchBaseQueryError; data?: undefined };

type ProjectBrowserBaseQuery = (
  arg: FetchArgs,
) => ProjectBrowserQueryResult | PromiseLike<ProjectBrowserQueryResult>;

async function postProjectBrowser<T>(
  api: { getState: () => unknown },
  baseQuery: ProjectBrowserBaseQuery,
  path: string,
  body: unknown,
  normalize: (value: unknown) => T | null,
): Promise<{ data: T } | { error: FetchBaseQueryError }> {
  const state = api.getState() as RootState;
  const response = await baseQuery({
    url: buildApiUrlFromState(state, path),
    method: "POST",
    body,
  });
  if (response.error) return { error: response.error };
  const data = normalize(response.data);
  if (data === null)
    return {
      error: {
        status: "CUSTOM_ERROR",
        error: `Unexpected response from ${path}`,
      },
    };
  return { data };
}

const acceptAnyBody = (): true => true;

export const projectBrowserStatusPublishing = createAction<string>(
  "browserApi/projectBrowserStatusPublishing",
);

async function publishProjectBrowserStatus(
  _args: unknown,
  {
    dispatch,
    queryFulfilled,
    requestId,
  }: {
    dispatch: (action: unknown) => unknown;
    queryFulfilled: Promise<{ data: ProjectBrowserStatus }>;
    requestId: string;
  },
) {
  try {
    const { data } = await queryFulfilled;
    dispatch(projectBrowserStatusPublishing(requestId));
    dispatch(
      browserApi.util.upsertQueryData("projectBrowserStatus", undefined, data),
    );
  } catch {
    return;
  }
}

export const browserApi = createApi({
  reducerPath: "browserApi",
  tagTypes: ["BROWSER"],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, api) => {
      const getState = api.getState as () => RootState;
      const state = getState();
      const token = state.config.apiKey;
      headers.set("credentials", "same-origin");
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    browserStart: builder.mutation<BrowserStartResponse, BrowserStartRequest>({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_START);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserStartResponse };
      },
    }),
    browserStop: builder.mutation<BrowserStopResponse, BrowserStopRequest>({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_STOP);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserStopResponse };
      },
    }),
    browserScreenshot: builder.mutation<
      BrowserScreenshotResponse,
      BrowserScreenshotRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_SCREENSHOT);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserScreenshotResponse };
      },
    }),
    browserContext: builder.mutation<
      BrowserContextResponse,
      BrowserContextRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_CONTEXT);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserContextResponse };
      },
    }),
    browserCurl: builder.mutation<BrowserCurlResponse, BrowserCurlRequest>({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_CURL);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserCurlResponse };
      },
    }),
    browserElementPick: builder.mutation<
      BrowserElementPickResponse,
      BrowserElementPickRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_ELEMENT_PICK);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserElementPickResponse };
      },
    }),
    browserElementPickResult: builder.mutation<
      BrowserElementPickResultResponse,
      BrowserElementPickResultRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_ELEMENT_PICK_RESULT);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserElementPickResultResponse };
      },
    }),
    browserElementPickCancel: builder.mutation<
      BrowserElementPickCancelResponse,
      BrowserElementPickCancelRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_ELEMENT_PICK_CANCEL);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserElementPickCancelResponse };
      },
    }),
    browserRecordAnimation: builder.mutation<
      BrowserRecordAnimationResponse,
      BrowserRecordAnimationRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_RECORD_ANIMATION);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserRecordAnimationResponse };
      },
    }),
    browserHandoff: builder.mutation<
      BrowserHandoffResponse,
      BrowserHandoffRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_HANDOFF);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserHandoffResponse };
      },
    }),
    browserStatus: builder.mutation<
      BrowserStatusResponse,
      BrowserStatusRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_STATUS);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserStatusResponse };
      },
    }),
    browserAnnotateStart: builder.mutation<
      BrowserAnnotateStartResponse,
      BrowserAnnotateStartRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_ANNOTATE_START);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserAnnotateStartResponse };
      },
    }),
    browserAnnotateResult: builder.mutation<
      BrowserAnnotateResultResponse,
      BrowserAnnotateResultRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_ANNOTATE_RESULT);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserAnnotateResultResponse };
      },
    }),
    browserAnnotateClear: builder.mutation<
      BrowserAnnotateClearResponse,
      BrowserAnnotateClearRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_ANNOTATE_CLEAR);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserAnnotateClearResponse };
      },
    }),
    browserContextEstimate: builder.mutation<
      BrowserContextEstimateResponse,
      BrowserContextEstimateRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_CONTEXT_ESTIMATE);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserContextEstimateResponse };
      },
    }),
    browserAction: builder.mutation<
      BrowserActionResponse,
      BrowserActionRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, BROWSER_ACTION);
        const response = await baseQuery({
          url,
          method: "POST",
          body: args,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as BrowserActionResponse };
      },
    }),
    projectBrowserStatus: builder.query<ProjectBrowserStatus, undefined>({
      queryFn: (_args, api, _extraOptions, baseQuery) =>
        postProjectBrowser(
          api,
          baseQuery,
          BROWSER_PROJECT_STATUS,
          {},
          normalizeProjectBrowserStatus,
        ),
    }),
    projectBrowserStart: builder.mutation<ProjectBrowserStatus, undefined>({
      queryFn: (_args, api, _extraOptions, baseQuery) =>
        postProjectBrowser(
          api,
          baseQuery,
          BROWSER_PROJECT_START,
          {},
          normalizeProjectBrowserStatus,
        ),
      onQueryStarted: publishProjectBrowserStatus,
    }),
    projectBrowserTab: builder.mutation<
      ProjectBrowserStatus,
      ProjectBrowserTabRequest
    >({
      queryFn: (args, api, _extraOptions, baseQuery) =>
        postProjectBrowser(
          api,
          baseQuery,
          BROWSER_PROJECT_TAB,
          args,
          normalizeProjectBrowserStatus,
        ),
      onQueryStarted: publishProjectBrowserStatus,
    }),
    projectBrowserNavigate: builder.mutation<
      ProjectBrowserStatus,
      ProjectBrowserNavigateRequest
    >({
      queryFn: (args, api, _extraOptions, baseQuery) =>
        postProjectBrowser(
          api,
          baseQuery,
          BROWSER_PROJECT_NAVIGATE,
          args,
          normalizeProjectBrowserStatus,
        ),
      onQueryStarted: publishProjectBrowserStatus,
    }),
    projectBrowserInput: builder.mutation<true, ProjectBrowserInputRequest>({
      queryFn: (args, api, _extraOptions, baseQuery) =>
        postProjectBrowser(
          api,
          baseQuery,
          BROWSER_PROJECT_INPUT,
          args,
          acceptAnyBody,
        ),
    }),
    projectBrowserResize: builder.mutation<true, ProjectBrowserResizeRequest>({
      queryFn: (args, api, _extraOptions, baseQuery) =>
        postProjectBrowser(
          api,
          baseQuery,
          BROWSER_PROJECT_RESIZE,
          args,
          acceptAnyBody,
        ),
    }),
    projectBrowserFrame: builder.mutation<
      ProjectBrowserFrame,
      ProjectBrowserFrameRequest
    >({
      queryFn: (args, api, _extraOptions, baseQuery) =>
        postProjectBrowser(
          api,
          baseQuery,
          BROWSER_PROJECT_FRAME,
          args,
          normalizeProjectBrowserFrame,
        ),
    }),
  }),
});
