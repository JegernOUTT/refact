import { useEffect, useRef, useState } from "react";
import { useStore } from "react-redux";
import type { AppStore } from "../app/store";
import { change, popBackTo, push } from "../features/Pages/pagesSlice";
import { selectWorkspaceSection } from "../features/Sidebar/sidebarSlice";
import {
  openFileInFilesPanel,
  validateAbsoluteFilePath,
} from "../features/Workspace/FilesPanel/filesPanelSlice";
import { setContentCard } from "../features/Workspace/workspaceSlice";
import { getEngineEndpointIdentity } from "../services/refact/apiUrl";
import {
  buildContentRoute,
  contentRoutePage,
  parseContentRoute,
  selectContentRoute,
} from "../utils/contentRoutes";
import { useAppSelector } from "./useAppSelector";
import { useConfig } from "./useConfig";
import { useInternalNavigation } from "./useInternalNavigation";

const useAppStore = useStore.withTypes<AppStore>();
function readLocation() {
  const route = parseContentRoute(window.location.hash);
  if (window.location.hash) return route;
  const id = new URLSearchParams(window.location.search).get("chat");
  return id
    ? parseContentRoute(`#/chat/${encodeURIComponent(id.trim())}`)
    : null;
}

export function useContentRouting(ready: boolean, restored: boolean) {
  const store = useAppStore();
  const config = useConfig();
  const enabled = config.host === "web";
  const [request, setRequest] = useState(() => ({
    route: readLocation(),
    revision: 0,
  }));
  const [initialTarget] = useState(() =>
    Boolean(window.location.hash || readLocation()),
  );
  const [applying, setApplying] = useState(initialTarget);
  const [errorDismissed, setErrorDismissed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [workspaceWaitExpired, setWorkspaceWaitExpired] = useState(false);
  const workspaceLoaded =
    useAppSelector(selectWorkspaceSection).status !== "loading";
  useEffect(() => {
    if (!enabled || !ready || workspaceLoaded) return;
    const timer = setTimeout(() => setWorkspaceWaitExpired(true), 8000);
    return () => clearTimeout(timer);
  }, [enabled, ready, workspaceLoaded]);
  const routeHash = useAppSelector((state) => {
    const route = selectContentRoute(state);
    return route ? buildContentRoute(route) : "";
  });
  const projectScope = useAppSelector((state) =>
    JSON.stringify(state.current_project),
  );
  const scope = `${getEngineEndpointIdentity(config)}:${projectScope}:${
    request.revision
  }`;
  const navigation = useInternalNavigation({ scope });
  const navigate = useRef(navigation.navigate);
  navigate.current = navigation.navigate;
  const serial = useRef(Promise.resolve());
  const revision = useRef(0);
  const lastWritten = useRef("");
  const initialized = useRef(false);
  const suppress = useRef(initialTarget);
  const lastLocation = useRef(window.location.href);

  useEffect(() => {
    if (!enabled) return;
    lastLocation.current = window.location.href;
    const onLocation = () => {
      if (lastLocation.current === window.location.href) return;
      lastLocation.current = window.location.href;
      revision.current += 1;
      suppress.current = true;
      setApplying(true);
      setRequest({ route: readLocation(), revision: revision.current });
    };
    window.addEventListener("popstate", onLocation);
    window.addEventListener("hashchange", onLocation);
    return () => {
      window.removeEventListener("popstate", onLocation);
      window.removeEventListener("hashchange", onLocation);
    };
  }, [enabled]);

  const canApply =
    enabled &&
    ready &&
    (workspaceLoaded || workspaceWaitExpired) &&
    (initialTarget || restored);
  useEffect(() => {
    if (!canApply) return;
    const lifecycle = { cancelled: false };
    const isCancelled = () => lifecycle.cancelled;
    suppress.current = true;
    setApplying(true);
    serial.current = serial.current.then(async () => {
      if (isCancelled()) return;
      const route =
        request.revision === 0 &&
        !window.location.hash &&
        !new URLSearchParams(window.location.search).has("chat")
          ? null
          : request.route;
      let opened = false;
      setErrorDismissed(false);
      setError(null);
      const focused = selectContentRoute(store.getState());
      if (
        route &&
        focused &&
        buildContentRoute(route) === buildContentRoute(focused)
      )
        opened = true;
      else if (route?.type === "chat" || route?.type === "task") {
        opened = await navigate.current(
          `refact://${route.type}/${route.id}${
            route.type === "task" && route.cardId ? `/card/${route.cardId}` : ""
          }`,
        );
        if (opened && route.type === "task")
          store.dispatch(
            setContentCard({ taskId: route.id, cardId: route.cardId ?? null }),
          );
      } else if (route?.type === "page") {
        const page = contentRoutePage(route.path);
        if (page) {
          store.dispatch(popBackTo({ name: "history" }));
          if (page.name !== "history") store.dispatch(push(page));
          opened = true;
        }
      } else if (route?.type === "file") {
        const path = validateAbsoluteFilePath(route.path);
        if (
          path.ok &&
          (config.capabilities?.filesPanel === true ||
            store.getState().workspace.panelsForced)
        ) {
          store.dispatch(
            openFileInFilesPanel({ path: path.path, manual: true }),
          );
          store.dispatch(change({ name: "chat" }));
          opened = true;
        }
      } else if (!window.location.hash) opened = true;
      if (isCancelled()) return;
      if (!opened)
        setError(
          "This content link is invalid, unavailable, or cannot be opened in this workspace.",
        );
      const current = selectContentRoute(store.getState());
      const hash = current ? buildContentRoute(current) : "";
      const canonical =
        opened && route?.type === "task"
          ? buildContentRoute(route)
          : hash || (opened && route ? buildContentRoute(route) : "");
      if (canonical)
        window.history.replaceState(
          window.history.state,
          "",
          `${window.location.pathname}${canonical}`,
        );
      lastLocation.current = window.location.href;
      lastWritten.current = canonical;
      initialized.current = true;
      suppress.current = false;
      setApplying(false);
    });
    return () => {
      lifecycle.cancelled = true;
    };
  }, [
    canApply,
    initialTarget,
    request,
    store,
    config.capabilities?.filesPanel,
    scope,
  ]);

  useEffect(() => {
    if (
      !enabled ||
      !ready ||
      !restored ||
      applying ||
      suppress.current ||
      !initialized.current ||
      !routeHash
    )
      return;
    if (lastWritten.current === routeHash) return;
    window.history.pushState(
      window.history.state,
      "",
      `${window.location.pathname}${routeHash}`,
    );
    lastLocation.current = window.location.href;
    lastWritten.current = routeHash;
  }, [enabled, ready, restored, applying, routeHash]);

  return {
    hasInitialTarget: enabled && initialTarget,
    pending: enabled && applying,
    error: errorDismissed ? null : error ?? navigation.error,
    dismissError: () => setErrorDismissed(true),
  };
}
