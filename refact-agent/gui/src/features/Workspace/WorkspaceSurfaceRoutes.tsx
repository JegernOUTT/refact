import {
  createContext,
  useContext,
  useLayoutEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  change,
  selectCurrentPage,
  type BuddyPage,
  type TaskWorkspacePage,
} from "../Pages/pagesSlice";
import { openTab } from "./workspaceSlice";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import { resolveProjectRoot } from "../Chat/currentProject";

type RouteScope = { endpoint: string; project: string | null };
const canRetainContexts = (previous: RouteScope, current: RouteScope) =>
  previous.endpoint === current.endpoint &&
  (previous.project === null || previous.project === current.project);

type SurfaceRouteContext = BuddyPage | TaskWorkspacePage;
const RouteContext = createContext<Record<string, SurfaceRouteContext>>({});

// eslint-disable-next-line react-refresh/only-export-components
export function useWorkspaceSurfaceRouteContext(key?: string | null) {
  const contexts = useContext(RouteContext);
  return key ? contexts[key] : undefined;
}

export function WorkspaceSurfaceRoutes({ children }: { children: ReactNode }) {
  const dispatch = useAppDispatch();
  const page = useAppSelector(selectCurrentPage);
  const endpoint = useAppSelector((state) =>
    getEngineEndpointIdentity(state.config),
  );
  const project = useAppSelector((state) => {
    const { projectRoot, workspaceRoots, name } = state.current_project;
    const root = resolveProjectRoot(projectRoot, workspaceRoots);
    return root.length > 0 ||
      (workspaceRoots?.length ?? 0) > 0 ||
      name.trim().length > 0
      ? JSON.stringify([root, workspaceRoots ?? [], root ? "" : name.trim()])
      : null;
  });
  const scope = useMemo(() => ({ endpoint, project }), [endpoint, project]);
  const workspace = useAppSelector((state) => state.workspace);
  const [saved, setSaved] = useState<{
    scope: RouteScope;
    contexts: Record<string, SurfaceRouteContext>;
  }>({ scope, contexts: {} });
  const intent =
    page?.name === "task workspace" || page?.name === "buddy"
      ? page
      : undefined;
  const key =
    intent?.name === "task workspace"
      ? `task:${intent.taskId}`
      : intent
        ? "buddy:home"
        : undefined;
  const contexts = useMemo(
    () => ({
      ...(canRetainContexts(saved.scope, scope) ? saved.contexts : {}),
      ...(key && intent ? { [key]: intent } : {}),
    }),
    [saved, scope, key, intent],
  );

  useLayoutEffect(() => {
    if (!key || !intent) return;
    setSaved({ scope, contexts });
    dispatch(openTab(key));
    dispatch(change({ name: "chat" }));
  }, [dispatch, key, intent, scope, contexts]);

  useLayoutEffect(() => {
    if (intent) return;
    if (
      saved.scope.project === null &&
      scope.project !== null &&
      canRetainContexts(saved.scope, scope)
    ) {
      Object.keys(saved.contexts).forEach((id) => dispatch(openTab(id)));
      setSaved({ scope, contexts: saved.contexts });
      return;
    }
    const owned = new Set<string>();
    const visit = (node: import("../ChatPanes/panesTree").PaneNode) => {
      if (node.kind === "leaf") node.tabIds.forEach((id) => owned.add(id));
      else node.children.forEach(visit);
    };
    Object.values(workspace.groups).forEach((group) => {
      if (group) visit(group.root);
    });
    setSaved((previous) => {
      const entries = Object.entries(
        canRetainContexts(previous.scope, scope) ? previous.contexts : {},
      ).filter(([id]) => owned.has(id));
      if (
        previous.scope === scope &&
        entries.length === Object.keys(previous.contexts).length
      )
        return previous;
      return { scope, contexts: Object.fromEntries(entries) };
    });
  }, [workspace.groups, scope, intent, saved, dispatch]);

  return (
    <RouteContext.Provider value={contexts}>{children}</RouteContext.Provider>
  );
}
