import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import {
  BROWSER_START,
  BROWSER_STOP,
  BROWSER_SCREENSHOT,
  BROWSER_CONTEXT,
  BROWSER_CURL,
  BROWSER_ELEMENT_PICK,
  BROWSER_RECORD_ANIMATION,
  BROWSER_HANDOFF,
  BROWSER_STATUS,
} from "./consts";

export type BrowserStartRequest = {
  chat_id: string;
};

export type BrowserStartResponse = {
  runtime_id: string;
  success: boolean;
};

export type BrowserStopRequest = {
  chat_id: string;
};

export type BrowserStopResponse = {
  success: boolean;
};

export type BrowserScreenshotRequest = {
  chat_id: string;
  full_page: boolean;
};

export type BrowserScreenshotResponse = {
  mime: string;
  data: string;
};

export type BrowserContextRequest = {
  chat_id: string;
  fields: string[];
};

export type BrowserContextResponse = {
  content: string;
};

export type BrowserCurlRequest = {
  chat_id: string;
};

export type BrowserCurlResponse = {
  curl_command: string;
};

export type BrowserElementPickRequest = {
  chat_id: string;
};

export type BrowserElementPickResponse = {
  selector: string;
  text: string;
  bbox: { x: number; y: number; width: number; height: number };
};

export type BrowserRecordAnimationRequest = {
  chat_id: string;
};

export type BrowserRecordAnimationResponse = {
  frames: { mime: string; data: string }[];
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
  }),
});
