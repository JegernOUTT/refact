import type { RootState } from "../../app/store";
import { isRecord } from "../../utils/isRecord";
import { buildApiUrlFromState } from "./apiUrl";
import { filesApi } from "./files";

export type ArtifactResolveRequest = {
  chatId: string;
  relPath: string;
};

export type ArtifactResolveResponse = {
  chat_id: string;
  rel_path: string;
  path: string;
  artifacts_dir: string;
  exists: boolean;
  bytes: number | null;
  modified_at_ms: number | null;
};

const isNullableCount = (value: unknown): value is number | null =>
  value === null ||
  (typeof value === "number" && Number.isSafeInteger(value) && value >= 0);

export function isArtifactResolveResponse(
  value: unknown,
): value is ArtifactResolveResponse {
  return (
    isRecord(value) &&
    typeof value.chat_id === "string" &&
    typeof value.rel_path === "string" &&
    typeof value.path === "string" &&
    value.path.length > 0 &&
    typeof value.artifacts_dir === "string" &&
    typeof value.exists === "boolean" &&
    isNullableCount(value.bytes) &&
    isNullableCount(value.modified_at_ms)
  );
}

export const artifactsApi = filesApi
  .enhanceEndpoints({ addTagTypes: ["Artifact"] })
  .injectEndpoints({
    endpoints: (builder) => ({
      resolveArtifact: builder.query<
        ArtifactResolveResponse,
        ArtifactResolveRequest
      >({
        queryFn: async ({ chatId, relPath }, api, _options, baseQuery) => {
          const result = await baseQuery({
            url: buildApiUrlFromState(
              api.getState() as RootState,
              "/v1/artifacts/resolve",
              { chat_id: chatId, path: relPath },
            ),
            credentials: "same-origin",
            redirect: "follow",
          });
          if (result.error) return { error: result.error };
          const data: unknown = result.data;
          if (
            !isArtifactResolveResponse(data) ||
            data.chat_id !== chatId ||
            data.rel_path !== relPath
          ) {
            return {
              error: {
                status: "CUSTOM_ERROR",
                error: "Invalid artifact resolve response",
              },
            };
          }
          return { data };
        },
        providesTags: (_result, _error, { chatId }) => [
          { type: "Artifact", id: chatId },
        ],
      }),
    }),
  });

export const { useResolveArtifactQuery } = artifactsApi;
