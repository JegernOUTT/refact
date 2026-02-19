import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import {
  BROWSER_START,
  BROWSER_STOP,
  BROWSER_SCREENSHOT,
  BROWSER_CONTEXT,
  BROWSER_CURL,
  BROWSER_ELEMENT_PICK,
  BROWSER_ELEMENT_PICK_RESULT,
  BROWSER_RECORD_ANIMATION,
  BROWSER_HANDOFF,
  BROWSER_STATUS,
  BROWSER_CONTEXT_ESTIMATE,
  BROWSER_ANNOTATE_START,
  BROWSER_ANNOTATE_RESULT,
  BROWSER_ANNOTATE_CLEAR,
} from "./consts";

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
  network: unknown[];
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
  url?: string;
  title?: string;
  tab_urls?: string[];
  idle_seconds?: number;
  idle_timeout?: number;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_START}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_STOP}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_SCREENSHOT}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_CONTEXT}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_CURL}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_ELEMENT_PICK}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_ELEMENT_PICK_RESULT}`;
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
    browserRecordAnimation: builder.mutation<
      BrowserRecordAnimationResponse,
      BrowserRecordAnimationRequest
    >({
      async queryFn(args, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_RECORD_ANIMATION}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_HANDOFF}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_STATUS}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_ANNOTATE_START}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_ANNOTATE_RESULT}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_ANNOTATE_CLEAR}`;
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
        const port = state.config.lspPort as unknown as number;
        const url = `http://127.0.0.1:${port}${BROWSER_CONTEXT_ESTIMATE}`;
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
  }),
});
