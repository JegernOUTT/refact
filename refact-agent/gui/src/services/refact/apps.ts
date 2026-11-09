import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type AppManifest = {
  schema_version: 1;
  app_id: string;
  name: string;
  description: string;
  icon?: string | null;
  entrypoint: string;
  sources: unknown[];
  actions: unknown[];
  permissions: unknown[];
};
export type AppIconGeometry = {
  tag:
    | "g"
    | "path"
    | "line"
    | "polyline"
    | "polygon"
    | "rect"
    | "circle"
    | "ellipse";
  attrs: Record<string, string>;
  children: AppIconGeometry[];
};
export type AppIconArtwork = {
  view_box: [number, number, number, number];
  nodes: AppIconGeometry[];
};

// Defense in depth for malformed/old servers: unknown attributes never become DOM props.
export function isAppIconArtwork(value: unknown): value is AppIconArtwork {
  if (
    !isAppRecord(value) ||
    !Array.isArray(value.view_box) ||
    value.view_box.length !== 4 ||
    !value.view_box.every(
      (n: unknown) =>
        typeof n === "number" && Number.isFinite(n) && Math.abs(n) <= 4096,
    ) ||
    Number(value.view_box[2]) <= 0 ||
    Number(value.view_box[3]) <= 0 ||
    !Array.isArray(value.nodes)
  )
    return false;
  let count = 0;
  let bytes = 0;
  const node = (n: unknown, depth: number): boolean => {
    if (
      ++count > 64 ||
      depth > 8 ||
      !isAppRecord(n) ||
      typeof n.tag !== "string" ||
      ![
        "g",
        "path",
        "line",
        "polyline",
        "polygon",
        "rect",
        "circle",
        "ellipse",
      ].includes(n.tag) ||
      !isAppRecord(n.attrs) ||
      !Array.isArray(n.children)
    )
      return false;
    if (n.tag !== "g" && n.children.length !== 0) return false;
    for (const [key, v] of Object.entries(n.attrs)) {
      if (
        typeof v !== "string" ||
        v.length > 4096 ||
        (bytes += v.length) > 16384
      )
        return false;
      if (key === "fill" || key === "stroke") {
        if (
          !/^(?:none|currentColor|#[0-9a-fA-F]{3}|#[0-9a-fA-F]{6}|#[0-9a-fA-F]{8})$/.test(
            v,
          )
        )
          return false;
      } else if (key === "stroke-linecap") {
        if (!["butt", "round", "square"].includes(v)) return false;
      } else if (key === "stroke-linejoin") {
        if (!["miter", "round", "bevel"].includes(v)) return false;
      } else if (key === "fill-rule" || key === "clip-rule") {
        if (!["nonzero", "evenodd"].includes(v)) return false;
      } else if (key === "d" && n.tag === "path") {
        if (!/^[Mm][MmZzLlHhVvCcSsQqTtAa0-9eE+.,\s-]+$/.test(v)) return false;
      } else if (key === "points" && ["polyline", "polygon"].includes(n.tag)) {
        if (!/^[0-9eE+.,\s-]+$/.test(v)) return false;
      } else if (
        ![
          "x",
          "y",
          "x1",
          "y1",
          "x2",
          "y2",
          "cx",
          "cy",
          "r",
          "rx",
          "ry",
          "width",
          "height",
          "stroke-width",
          "stroke-miterlimit",
          "opacity",
          "fill-opacity",
          "stroke-opacity",
        ].includes(key) ||
        v.trim() === "" ||
        !Number.isFinite(Number(v)) ||
        Math.abs(Number(v)) > 4096
      )
        return false;
    }
    return n.children.every((child: unknown) => node(child, depth + 1));
  };
  return value.nodes.every((n: unknown) => node(n, 0));
}
export type CatalogApp = {
  slug: string;
  icon_artwork?: AppIconArtwork | null;
  manifest: AppManifest;
  revision: string;
  active_revision: string | null;
};
export type AppCatalog = { apps: CatalogApp[]; diagnostics: unknown[] };
export type AppViewIdentity = {
  view_id: string;
  view_token: string;
  generation: string;
  revision: string;
};
export type AppStateScope =
  | { type: "shared" }
  | { type: "binding"; binding_id: string }
  | { type: "standalone" };
export type AppCommand =
  | { action: "get" | "details"; app_id: string }
  | { action: "remove"; app_id: string; expected_revision: string }
  | { action: "source_list"; app_id: string }
  | {
      action: "context_stage";
      view_id: string;
      view_token: string;
      text: string;
      label?: string;
    }
  | {
      action: "view_register";
      app_id: string;
      chat_id?: string;
      binding_id?: string;
    }
  | {
      action: "view_unregister";
      view_id: string;
      view_token: string;
    }
  | {
      action: "view_heartbeat";
      view_id: string;
      view_token: string;
      visible?: boolean;
      focused?: boolean;
      ready?: boolean;
    }
  | {
      action: "event_emit";
      view_id: string;
      view_token: string;
      channel: string;
      data: unknown;
    }
  | {
      action: "action_request";
      view_id: string;
      view_token: string;
      request_id: string;
      action_id: string;
      payload: unknown;
    }
  | {
      action: "capture_response";
      view_id: string;
      view_token: string;
      request_id: string;
      data_url: string | null;
      metadata?: unknown;
      error?: string;
    }
  | {
      action: "state_get" | "state_set";
      app_id: string;
      scope: AppStateScope;
      chat_id?: string;
      view_id?: string;
      view_token?: string;
      expected_revision?: number;
      value?: unknown;
    };
export type AppDocument = {
  app_id: string;
  revision: string;
  html: string;
  files?: Record<string, string>;
  manifest?: AppManifest;
  state: { revision: number; value: unknown };
  sources: unknown[];
};
export const isAppRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

export function isAppManifest(value: unknown): value is AppManifest {
  return (
    isAppRecord(value) &&
    value.schema_version === 1 &&
    typeof value.app_id === "string" &&
    value.app_id.length > 0 &&
    typeof value.name === "string" &&
    typeof value.description === "string" &&
    typeof value.entrypoint === "string" &&
    (value.icon === undefined ||
      value.icon === null ||
      typeof value.icon === "string") &&
    Array.isArray(value.sources) &&
    Array.isArray(value.actions) &&
    Array.isArray(value.permissions)
  );
}
export function isAppCatalog(value: unknown): value is AppCatalog {
  return (
    isAppRecord(value) &&
    Array.isArray(value.diagnostics) &&
    Array.isArray(value.apps) &&
    value.apps.every(
      (app: unknown) =>
        isAppRecord(app) &&
        typeof app.slug === "string" &&
        isAppManifest(app.manifest) &&
        typeof app.revision === "string" &&
        (app.active_revision === null ||
          typeof app.active_revision === "string"),
    )
  );
}
export function isAppDocument(value: unknown): value is AppDocument {
  return (
    isAppRecord(value) &&
    typeof value.app_id === "string" &&
    typeof value.revision === "string" &&
    typeof value.html === "string" &&
    (value.files === undefined ||
      (isAppRecord(value.files) &&
        Object.values(value.files).every(
          (file) => typeof file === "string",
        ))) &&
    (value.manifest === undefined || isAppManifest(value.manifest)) &&
    Array.isArray(value.sources) &&
    isAppRecord(value.state) &&
    Number.isSafeInteger(value.state.revision) &&
    Number(value.state.revision) >= 0 &&
    "value" in value.state
  );
}
const invalidResponse = {
  status: "CUSTOM_ERROR" as const,
  error: "Invalid app response",
};

const optionalId = (value: string | null | undefined): string | undefined =>
  value === "" ? undefined : value ?? undefined;

export const appsApi = createApi({
  reducerPath: "appsApi",
  tagTypes: ["Apps", "AppDocument"],
  baseQuery: fetchBaseQuery({
    prepareHeaders(headers, api) {
      const token = (api.getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  endpoints: (builder) => ({
    appCommand: builder.mutation<
      unknown,
      AppCommand & { hostChatId?: string | null }
    >({
      invalidatesTags: (result, error, input) =>
        !error &&
        input.action === "remove" &&
        isAppRecord(result) &&
        result.removed === true
          ? ["Apps"]
          : [],
      async queryFn(input, api, _options, baseQuery) {
        const { hostChatId, ...command } = input;
        const response = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/apps/commands",
            {
              chat_id: optionalId(
                hostChatId !== undefined
                  ? hostChatId
                  : "chat_id" in command
                    ? command.chat_id
                    : undefined,
              ),
            },
          ),
          method: "POST",
          body: command,
        });
        return response.error
          ? { error: response.error }
          : { data: response.data };
      },
    }),
    getApps: builder.query<AppCatalog, string | undefined>({
      providesTags: ["Apps"],
      async queryFn(chatId, api, _options, baseQuery) {
        const response = await baseQuery({
          url: buildApiUrlFromState(api.getState() as RootState, "/v1/apps", {
            chat_id: optionalId(chatId),
          }),
        });
        if (response.error) return { error: response.error };
        return isAppCatalog(response.data)
          ? { data: response.data }
          : { error: invalidResponse };
      },
    }),
    getAppDocument: builder.query<
      AppDocument,
      { appId: string; chatId?: string; bindingId?: string }
    >({
      providesTags: (_result, _error, query) => [
        { type: "AppDocument", id: query.appId },
      ],
      async queryFn(query, api, _options, baseQuery) {
        const response = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            `/v1/apps/${encodeURIComponent(query.appId)}/document`,
            {
              chat_id: optionalId(query.chatId),
              binding_id: optionalId(query.bindingId),
            },
          ),
        });
        if (response.error) return { error: response.error };
        return isAppDocument(response.data) &&
          response.data.app_id === query.appId
          ? { data: response.data }
          : { error: invalidResponse };
      },
    }),
  }),
});
export const {
  useGetAppsQuery,
  useGetAppDocumentQuery,
  useAppCommandMutation,
} = appsApi;
