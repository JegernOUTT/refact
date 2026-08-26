import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export interface BrowserLaunchSettings {
  chrome_path: string;
  headless: boolean;
  chromium_sandbox: boolean;
  ignore_https_errors: boolean;
  mask_passwords: boolean;
  extra_args: string[];
  downloads_dir: string;
  proxy_server: string;
  proxy_bypass: string;
}

export interface BrowserLifecycleSettings {
  idle_timeout_secs: number;
  evict_idle_attached: boolean;
  monitor_interval_secs: number;
  relaunch_settle_ms: number;
}

export interface BrowserViewportGroup {
  width: number;
  height: number;
  scale_factor: number;
}

export interface BrowserViewportSettings {
  desktop: BrowserViewportGroup;
  mobile: BrowserViewportGroup;
  tablet: BrowserViewportGroup;
}

export interface BrowserTimingSettings {
  default_wait_timeout_ms: number;
  max_wait_timeout_ms: number;
  default_poll_interval_ms: number;
}

export interface BrowserCaptureSettings {
  default_aria_snapshot_chars: number;
  max_aria_snapshot_chars: number;
  max_dom_snapshot_chars: number;
  max_inline_snapshot_bytes: number;
  max_extract_links: number;
  max_extract_table_rows: number;
  default_all_texts: number;
}

export interface BrowserSettings {
  launch: BrowserLaunchSettings;
  lifecycle: BrowserLifecycleSettings;
  viewport: BrowserViewportSettings;
  timing: BrowserTimingSettings;
  capture: BrowserCaptureSettings;
}

export const browserSettingsApi = createApi({
  reducerPath: "browserSettingsApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["BrowserSettings"],
  endpoints: (builder) => ({
    getBrowserSettings: builder.query<BrowserSettings, undefined>({
      queryFn: async (_argument, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/browser-settings"),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BrowserSettings };
      },
      providesTags: [{ type: "BrowserSettings", id: "settings" }],
    }),
    saveBrowserSettings: builder.mutation<BrowserSettings, BrowserSettings>({
      queryFn: async (settings, api, _options, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/browser-settings"),
          method: "POST",
          body: settings,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BrowserSettings };
      },
      invalidatesTags: [{ type: "BrowserSettings", id: "settings" }],
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export const { useGetBrowserSettingsQuery, useSaveBrowserSettingsMutation } =
  browserSettingsApi;
