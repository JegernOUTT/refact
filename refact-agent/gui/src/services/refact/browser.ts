import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
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

export type BrowserRouteInfo = {
  pattern: BrowserUrlPattern;
  handler: BrowserRouteHandler;
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

export type BrowserActionResponse = {
  ok: boolean;
  steps: BrowserExecutionStep[];
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
  }),
});
