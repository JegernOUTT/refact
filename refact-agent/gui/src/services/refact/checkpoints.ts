import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { PREVIEW_CHECKPOINTS, RESTORE_CHECKPOINTS } from "./consts";
import { buildApiUrlFromState } from "./apiUrl";
import {
  isPreviewCheckpointsResponse,
  isRestoreCheckpointsResponse,
  PreviewCheckpointsPayload,
  PreviewCheckpointsResponse,
  RestoreCheckpointsPayload,
  RestoreCheckpointsResponse,
} from "../../features/Checkpoints/types";

export const checkpointsApi = createApi({
  reducerPath: "checkpointsApi",
  tagTypes: ["CHECKPOINTS"],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, api) => {
      const getState = api.getState as () => RootState;
      const state = getState();
      const token = state.config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    previewCheckpoints: builder.mutation<
      PreviewCheckpointsResponse,
      PreviewCheckpointsPayload
    >({
      async queryFn(args, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const { checkpoints, chat_id, chat_mode } = args;
        const url = buildApiUrlFromState(state, PREVIEW_CHECKPOINTS);

        const result = await baseQuery({
          url,
          credentials: "same-origin",
          redirect: "follow",
          method: "POST",
          body: {
            meta: {
              chat_id,
              chat_mode: chat_mode ?? "EXPLORE",
            },
            checkpoints,
          },
        });

        if (result.error) return { error: result.error };

        if (!isPreviewCheckpointsResponse(result.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Failed to parse preview checkpoints response",
              data: result.data,
            },
          };
        }

        return { data: result.data };
      },
    }),
    restoreCheckpoints: builder.mutation<
      RestoreCheckpointsResponse,
      RestoreCheckpointsPayload
    >({
      async queryFn(args, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const { checkpoints, chat_id, chat_mode } = args;
        const url = buildApiUrlFromState(state, RESTORE_CHECKPOINTS);

        const result = await baseQuery({
          url,
          credentials: "same-origin",
          redirect: "follow",
          method: "POST",
          body: {
            meta: {
              chat_id,
              chat_mode: chat_mode ?? "EXPLORE",
            },
            checkpoints,
          },
        });

        if (result.error) return { error: result.error };
        if (!isRestoreCheckpointsResponse(result.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Failed to parse restored checkpoints response",
              data: result.data,
            },
          };
        }

        return { data: result.data };
      },
    }),
  }),
});
