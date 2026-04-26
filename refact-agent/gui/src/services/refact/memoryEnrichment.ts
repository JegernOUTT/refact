import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import type { ManualPreviewItem } from "../../features/Chat/Thread/types";
import { selectApiKey } from "../../features/Config/configSlice";

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
      query: ({ chatId, text, port }) => ({
        url: `http://127.0.0.1:${port}/v1/chats/${chatId}/memory-enrichment/preview`,
        method: "POST",
        body: { text },
      }),
      transformResponse: (
        response: MemoryEnrichmentPreviewResponse,
      ): PreviewResult => ({
        rewrittenText: response.rewritten_text ?? "",
        items: response.items.map(
          (item): ManualPreviewItem => ({
            kind: item.kind,
            label: item.label,
            context_file: item.context_file,
          }),
        ),
      }),
    }),
  }),
});

export const { usePreviewMemoryEnrichmentMutation } = memoryEnrichmentApi;
