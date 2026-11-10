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
  type ContentRoute,
} from "../utils/contentRoutes";
import { useAppSelector } from "./useAppSelector";
import { useConfig } from "./useConfig";
import { useInternalNavigation } from "./useInternalNavigation";

const useAppStore = useStore.withTypes<AppStore>();
const ID_DISPLAY_LENGTH = 8;
const NAVIGATION_ERROR_PREFIX = "Could not open link: ";
const DEFAULT_REASON =
  "it is unavailable or cannot be opened in this workspace";
const CAPABILITY_WAIT_MS = 10_000;

type LinkFailure = {
  subject: string | null;
  reason: string | null;
  fromNavigation: boolean;
};

function shortId(id: string) {
  return id.length > ID_DISPLAY_LENGTH + 1
    ? `${id.slice(0, ID_DISPLAY_LENGTH)}…`
    : id;
}

function describeFailure(
  route: ContentRoute | null,
  reason: string | null,
): LinkFailure {
  if (!route) return { subject: null, reason, fromNavigation: false };
  if (route.type === "file") {
    const name = route.path.split(/[\\/]/u).filter(Boolean).at(-1);
    return {
      subject: `File ${name ?? route.path}`,
      reason,
      fromNavigation: false,
    };
  }
  if (route.type === "page")
    return { subject: `Page ${route.path}`, reason, fromNavigation: false };
  const kind = { chat: "Chat", task: "Task", app: "App" }[route.type];
  const card =
    route.type === "task" && route.cardId
      ? ` card ${shortId(route.cardId)}`
      : "";
  return {
    subject: `${kind} ${shortId(route.id)}${card}`,
    reason,
    fromNavigation: route.type === "chat" || route.type === "task",
  };
}

function navigationReason(error: string | null) {
  if (!error) return null;
  const text = error.startsWith(NAVIGATION_ERROR_PREFIX)
    ? error.slice(NAVIGATION_ERROR_PREFIX.length)
    : error;
  return text.trim().replace(/\.$/u, "") || null;
}

function formatFailure(failure: LinkFailure, navigationError: string | null) {
  if (!failure.subject)
    return "The address was not recognised as a content link.";
  const reason = navigationError ?? failure.reason ?? DEFAULT_REASON;
  return `${failure.subject} could not be opened: ${reason}.`;
}

function queryKey(segment: string) {
  const raw = segment.split("=", 1)[0].replace(/\+/gu, " ");
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

function addressAfterFailure(dropChat: boolean, hash: string) {
  const search = window.location.search;
  const query = dropChat
    ? search
        .replace(/^\?/u, "")
        .split("&")
        .filter((segment) => queryKey(segment) !== "chat")
        .join("&")
    : search.replace(/^\?/u, "");
  return `${window.location.pathname}${query ? `?${query}` : ""}${hash}`;
}

function navigationSource(state: ReturnType<AppStore["getState"]>) {
  return [
    state.pages.at(-1),
    state.chat.current_thread_id,
    state.workspace.activeTabId,
  ] as const;
}

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
  const [failure, setFailure] = useState<LinkFailure | null>(null);
  const failedRevision = useRef<number | null>(null);
  const awaitingCapability = useRef<number | null>(null);
  const stopWaiting = useRef<(() => void) | null>(null);
  const endCapabilityWait = (failed: boolean) => {
    stopWaiting.current?.();
    stopWaiting.current = null;
    if (failed && awaitingCapability.current !== null)
      failedRevision.current = awaitingCapability.current;
    awaitingCapability.current = null;
  };
  const endWait = useRef(endCapabilityWait);
  endWait.current = endCapabilityWait;
  useEffect(() => () => endWait.current(false), []);
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
  const navigate = useRef(navigation.navigateWithOutcome);
  navigate.current = navigation.navigateWithOutcome;
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
      endWait.current(true);
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
      const finish = () => {
        lastLocation.current = window.location.href;
        initialized.current = true;
        suppress.current = false;
        setApplying(false);
      };
      const filesAvailable =
        config.capabilities?.filesPanel === true ||
        store.getState().workspace.panelsForced;
      if (
        failedRevision.current === request.revision ||
        (awaitingCapability.current === request.revision && !filesAvailable)
      ) {
        finish();
        return;
      }
      const route =
        request.revision === 0 &&
        awaitingCapability.current !== request.revision &&
        !window.location.hash &&
        !new URLSearchParams(window.location.search).has("chat")
          ? null
          : request.route;
      const fromChatQuery =
        !window.location.hash &&
        new URLSearchParams(window.location.search).has("chat");
      endWait.current(false);
      let opened = false;
      let superseded = false;
      let reason: string | null = null;
      setFailure(null);
      const focused = selectContentRoute(store.getState());
      if (
        route &&
        focused &&
        buildContentRoute(route) === buildContentRoute(focused)
      )
        opened = true;
      else if (route?.type === "chat" || route?.type === "task") {
        const outcome = await navigate.current(
          `refact://${route.type}/${route.id}${
            route.type === "task" && route.cardId ? `/card/${route.cardId}` : ""
          }`,
        );
        opened = outcome === "opened";
        superseded = outcome === "superseded" || outcome === "busy";
        if (superseded) failedRevision.current = request.revision;
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
        if (!path.ok) reason = "the path is not a valid absolute path";
        else if (filesAvailable) {
          store.dispatch(
            openFileInFilesPanel({ path: path.path, manual: true }),
          );
          store.dispatch(change({ name: "chat" }));
          opened = true;
        } else {
          reason = "files cannot be opened in this workspace";
          const waiting = request.revision;
          awaitingCapability.current = waiting;
          const source = navigationSource(store.getState());
          const expire = () => {
            if (awaitingCapability.current === waiting) endWait.current(true);
          };
          const unsubscribe = store.subscribe(() => {
            const latest = navigationSource(store.getState());
            if (source.some((value, index) => value !== latest[index]))
              expire();
          });
          const timer = setTimeout(expire, CAPABILITY_WAIT_MS);
          stopWaiting.current = () => {
            unsubscribe();
            clearTimeout(timer);
          };
        }
      } else if (!window.location.hash) opened = true;
      if (isCancelled()) return;
      if (!opened && !superseded) {
        if (awaitingCapability.current === null)
          failedRevision.current = request.revision;
        setFailure(describeFailure(route, reason));
      }
      const current = selectContentRoute(store.getState());
      const hash = current ? buildContentRoute(current) : "";
      const canonical =
        opened && route?.type === "task"
          ? buildContentRoute(route)
          : hash || (opened && route ? buildContentRoute(route) : "");
      if (!opened)
        window.history.replaceState(
          window.history.state,
          "",
          addressAfterFailure(fromChatQuery, canonical),
        );
      else if (canonical)
        window.history.replaceState(
          window.history.state,
          "",
          `${window.location.pathname}${canonical}`,
        );
      lastWritten.current = canonical;
      finish();
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

  const error = failure
    ? formatFailure(
        failure,
        failure.fromNavigation ? navigationReason(navigation.error) : null,
      )
    : null;
  return {
    hasInitialTarget: enabled && initialTarget,
    pending: enabled && applying,
    error,
    dismissError: () => {
      endWait.current(true);
      setFailure(null);
    },
  };
}
