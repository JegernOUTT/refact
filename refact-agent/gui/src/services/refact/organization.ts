import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type OrganizationItem = { kind: "chat" | "task"; id: string };
export type Taxon = { id: string; name: string; archived: boolean };
export type Assignment = {
  item: OrganizationItem;
  topic_id: string | null;
  tag_ids: string[];
  topic_locked: boolean;
  tags_locked: boolean;
};
export type Taxonomy = {
  revision: number;
  items: Taxon[];
  next_cursor: string | null;
};
export type Assignments = {
  revision: number;
  assignments: Assignment[];
  unavailable_items?: OrganizationItem[];
};
export type OrganizationOperation =
  | ({ op: "assign_item" } & Assignment)
  | { op: "create_topic" | "create_tag"; name: string };
export const organizationItemKey = (item: OrganizationItem) =>
  `${item.kind}:${item.id}`;

export const organizationApi = createApi({
  reducerPath: "organizationApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["Organization"],
  endpoints: (builder) => ({
    organizationTaxonomy: builder.query<
      Taxonomy,
      { scope: string; kind: "topics" | "tags" }
    >({
      providesTags: ["Organization"],
      queryFn: async ({ kind }, api, _options, baseQuery) => {
        for (let attempt = 0; attempt < 3; attempt++) {
          const items: Taxon[] = [];
          let cursor: string | null = null;
          let revision: number | undefined;
          let stale = false;
          const seen = new Set<string>();
          do {
            const params = new URLSearchParams({ kind, limit: "200" });
            if (cursor) params.set("cursor", cursor);
            const result = await baseQuery({
              url: buildApiUrlFromState(
                api.getState() as RootState,
                `/v1/organization/taxonomy?${params.toString()}`,
              ),
            });
            if (result.error) {
              if (result.error.status === 409) {
                stale = true;
                break;
              }
              return { error: result.error };
            }
            const page = result.data as Taxonomy;
            if (revision !== undefined && revision !== page.revision) {
              stale = true;
              break;
            }
            revision = page.revision;
            items.push(...page.items);
            cursor = page.next_cursor;
            if (cursor && seen.has(cursor))
              return {
                error: {
                  status: "CUSTOM_ERROR",
                  error: "Invalid organization cursor",
                },
              };
            if (cursor) seen.add(cursor);
          } while (cursor);
          if (!stale)
            return {
              data: { revision: revision ?? 0, items, next_cursor: null },
            };
        }
        return {
          error: {
            status: "CUSTOM_ERROR",
            error: "Organization changed while loading. Retry.",
          },
        };
      },
    }),
    organizationItems: builder.query<
      Assignments,
      { scope: string; items: OrganizationItem[] }
    >({
      providesTags: ["Organization"],
      queryFn: async ({ items }, api, _options, baseQuery) => {
        let isolationCalls = 0;
        for (let attempt = 0; attempt < 3; attempt++) {
          const assignments: Assignment[] = [];
          const unavailable_items: OrganizationItem[] = [];
          let revision: number | undefined;
          let stale = false;
          const batches = Array.from(
            { length: Math.ceil(items.length / 200) },
            (_, index) => ({
              items: items.slice(index * 200, (index + 1) * 200),
              isolation: false,
            }),
          );
          while (batches.length > 0) {
            const batch = batches.shift();
            if (!batch) break;
            if (batch.isolation) {
              if (isolationCalls >= 32) {
                unavailable_items.push(...batch.items);
                continue;
              }
              isolationCalls++;
            }
            const result = await baseQuery({
              url: buildApiUrlFromState(
                api.getState() as RootState,
                "/v1/organization/items",
              ),
              method: "POST",
              body: { items: batch.items },
            });
            if (result.error) {
              if (result.error.status !== 403) return { error: result.error };
              if (batch.items.length === 1)
                unavailable_items.push(...batch.items);
              else {
                const middle = Math.ceil(batch.items.length / 2);
                batches.unshift(
                  { items: batch.items.slice(0, middle), isolation: true },
                  { items: batch.items.slice(middle), isolation: true },
                );
              }
              continue;
            }
            const page = result.data as Assignments;
            if (revision !== undefined && revision !== page.revision) {
              stale = true;
              break;
            }
            revision = page.revision;
            assignments.push(...page.assignments);
          }
          if (!stale)
            return {
              data: { revision: revision ?? 0, assignments, unavailable_items },
            };
        }
        return {
          error: {
            status: "CUSTOM_ERROR",
            error: "Organization changed while loading. Retry.",
          },
        };
      },
    }),
    applyOrganization: builder.mutation<
      { revision: number; results: { op_index: number; id?: string }[] },
      {
        scope: string;
        expected_revision: number;
        request_id: string;
        operations: OrganizationOperation[];
      }
    >({
      invalidatesTags: ["Organization"],
      queryFn: async ({ scope: _scope, ...body }, api, _options, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/organization/apply",
          ),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return {
          data: result.data as {
            revision: number;
            results: { op_index: number; id?: string }[];
          },
        };
      },
    }),
  }),
});
export const {
  useOrganizationTaxonomyQuery,
  useOrganizationItemsQuery,
  useApplyOrganizationMutation,
} = organizationApi;
