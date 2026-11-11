import { createContext, useContext, useEffect, useMemo, useRef } from "react";
import type { RootState } from "../../../app/store";
import { useAppSelector } from "../../../hooks";
import { buildApiUrlFromState } from "../../../services/refact/apiUrl";
import {
  type Assignments,
  organizationApi,
  organizationItemKey,
  useOrganizationItemsQuery,
  useOrganizationTaxonomyQuery,
} from "../../../services/refact/organization";
import { selectApiKey } from "../../Config/configSlice";
import { selectBackendStatus } from "../../Connection";
import { selectStreamItems } from "../streamSelectors";

const organizationTaxonomySelect =
  organizationApi.endpoints.organizationTaxonomy.select;
const organizationItemsSelect =
  organizationApi.endpoints.organizationItems.select;

const isUnauthorized = (error: unknown) =>
  !!error &&
  typeof error === "object" &&
  "status" in error &&
  error.status === 401;

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
  const apiKey = useAppSelector(selectApiKey);
  const backendStatus = useAppSelector(selectBackendStatus);
  const topicsArgs = useMemo(
    () => ({ scope, kind: "topics" }) as const,
    [scope],
  );
  const tagsArgs = useMemo(() => ({ scope, kind: "tags" }) as const, [scope]);
  const itemsArgs = useMemo(() => ({ scope, items: keys }), [scope, keys]);
  const selectSuspended = useMemo(() => {
    const selectTopics = organizationTaxonomySelect(topicsArgs);
    const selectTags = organizationTaxonomySelect(tagsArgs);
    const selectItems = organizationItemsSelect(itemsArgs);
    return (state: RootState) =>
      isUnauthorized(selectTopics(state).error) ||
      isUnauthorized(selectTags(state).error) ||
      (itemsArgs.items.length > 0 && isUnauthorized(selectItems(state).error));
  }, [topicsArgs, tagsArgs, itemsArgs]);
  const suspended = useAppSelector(selectSuspended);
  const options = {
    pollingInterval: suspended ? 0 : 30000,
    refetchOnFocus: !suspended,
    refetchOnMountOrArgChange: true,
  };
  const topics = useOrganizationTaxonomyQuery(topicsArgs, options);
  const tags = useOrganizationTaxonomyQuery(tagsArgs, options);
  const assignments = useOrganizationItemsQuery(itemsArgs, {
    ...options,
    skip: keys.length === 0,
  });
  const retry = () => {
    void topics.refetch();
    void tags.refetch();
    if (keys.length) void assignments.refetch();
  };
  const latestRetry = useRef(retry);
  useEffect(() => {
    latestRetry.current = retry;
  });
  const lastAccess = useRef({ online: backendStatus === "online", apiKey });
  useEffect(() => {
    const online = backendStatus === "online";
    const previous = lastAccess.current;
    lastAccess.current = { online, apiKey };
    if ((online && !previous.online) || apiKey !== previous.apiKey)
      latestRetry.current();
  }, [backendStatus, apiKey]);
  const retained = useRef<{ scope: string; data: Assignments | undefined }>({
    scope,
    data: undefined,
  });
  if (retained.current.scope !== scope)
    retained.current = { scope, data: undefined };
  if (assignments.currentData) retained.current.data = assignments.currentData;
  const assignmentData = assignments.currentData ?? retained.current.data;
  const byKey = useMemo(
    () =>
      new Map(
        assignmentData?.assignments.map((assignment) => [
          organizationItemKey(assignment.item),
          assignment,
        ]),
      ),
    [assignmentData],
  );
  return {
    scope,
    topics,
    tags,
    assignments: { ...assignments, currentData: assignmentData },
    byKey,
    error: topics.isError || tags.isError || assignments.isError,
    retry,
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
