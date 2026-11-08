import { createContext, useContext, useMemo } from "react";
import { useAppSelector } from "../../../hooks";
import { buildApiUrlFromState } from "../../../services/refact/apiUrl";
import {
  organizationItemKey,
  useOrganizationItemsQuery,
  useOrganizationTaxonomyQuery,
} from "../../../services/refact/organization";
import { selectStreamItems } from "../streamSelectors";

export function useOrganizationData() {
  const scope = useAppSelector(
    (state) =>
      `${buildApiUrlFromState(state, "/v1/organization")}:${
        state.current_project.projectRoot ??
        state.current_project.workspaceRoots?.[0] ??
        state.current_project.name
      }`,
  );
  const items = useAppSelector(selectStreamItems);
  const keys = useMemo(
    () => [
      ...new Map(
        items.map((item) => {
          const key = item.organizationKey ?? { kind: item.kind, id: item.id };
          return [organizationItemKey(key), key] as const;
        }),
      ).values(),
    ],
    [items],
  );
  const options = {
    pollingInterval: 30000,
    refetchOnFocus: true,
    refetchOnMountOrArgChange: true,
  };
  const topics = useOrganizationTaxonomyQuery(
    { scope, kind: "topics" },
    options,
  );
  const tags = useOrganizationTaxonomyQuery({ scope, kind: "tags" }, options);
  const assignments = useOrganizationItemsQuery(
    { scope, items: keys },
    { ...options, skip: keys.length === 0 },
  );
  const byKey = useMemo(
    () =>
      new Map(
        assignments.currentData?.assignments.map((assignment) => [
          organizationItemKey(assignment.item),
          assignment,
        ]),
      ),
    [assignments.currentData],
  );
  return {
    scope,
    topics,
    tags,
    assignments,
    byKey,
    error: topics.isError || tags.isError || assignments.isError,
    retry: () => {
      void topics.refetch();
      void tags.refetch();
      if (keys.length) void assignments.refetch();
    },
  };
}

export const OrganizationContext = createContext<ReturnType<
  typeof useOrganizationData
> | null>(null);
export function useOrganization() {
  const value = useContext(OrganizationContext);
  if (!value) throw new Error("Organization context missing");
  return value;
}
