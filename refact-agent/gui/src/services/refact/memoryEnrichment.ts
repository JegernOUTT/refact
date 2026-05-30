import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import type { ManualPreviewItem } from "../../features/Chat/Thread/types";
import { selectApiKey } from "../../features/Config/configSlice";
import { buildApiUrlFromState } from "./apiUrl";

export type PreviewResult = {
  rewrittenText: string;
  items: ManualPreviewItem[];
};

export type MemoryEnrichmentPreviewRequest = {
  text: string;
};

/** Raw item shape as returned by the backend preview endpoint. */
type BackendEnrichmentItem = {
  kind: "memory" | "trajectory" | "file";
  label: string;
  context_file: {
    file_name: string;
    file_content: string;
    line1: number;
    line2: number;
    usefulness: number;
    skip_pp?: boolean;
    gradient_type?: number;
  };
};

export type MemoryEnrichmentPreviewResponse = {
  query_used: string;
  rewritten_text?: string;
  items: BackendEnrichmentItem[];
};

export const memoryEnrichmentApi = createApi({
  reducerPath: "memoryEnrichmentApi",
  baseQuery: fetchBaseQuery({
    baseUrl: "/",
    prepareHeaders: (headers, { getState }) => {
      const state = getState() as RootState;
      const apiKey = selectApiKey(state);
      if (apiKey) {
        headers.set("Authorization", `Bearer ${apiKey}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    previewMemoryEnrichment: builder.mutation<
      PreviewResult,
      { chatId: string; text: string; port: string | number }
    >({
      async queryFn({ chatId, text }, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `/v1/chats/${encodeURIComponent(chatId)}/memory-enrichment/preview`,
        );

        const response = await baseQuery({
          url,
          method: "POST",
          body: { text },
        });

        if (response.error) return { error: response.error };

        const data = response.data as MemoryEnrichmentPreviewResponse;
        return {
          data: {
            rewrittenText: data.rewritten_text ?? "",
            items: data.items.map(
              (item): ManualPreviewItem => ({
                kind: item.kind,
                label: item.label,
                context_file: item.context_file,
              }),
            ),
          },
        };
      },
    }),
  }),
});

export const { usePreviewMemoryEnrichmentMutation } = memoryEnrichmentApi;
