import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Flex } from "@radix-ui/themes";
import {
  createChatWithId,
  selectAllThreads,
  selectBackgroundAgentsByThread,
  selectChatId,
  selectIsStreaming,
  selectThread,
  switchToThread,
} from "./Chat";
import { WorkspaceView } from "./Workspace/WorkspaceView";

import {
  useAppSelector,
  useAppDispatch,
  useConfig,
  useEffectOnce,
  useEventsBusForIDE,
  useSidebarSubscription,
  useAllChatsSubscription,
  useProviderBootstrapState,
  useResizeObserverOnRef,
} from "../hooks";
import { useBrowserOnlineStatus } from "../hooks/useBrowserOnlineStatus";
import { store } from "../app/store";
import { Provider } from "react-redux";
import { Theme } from "../components/Theme";
import { useEventBusForWeb } from "../hooks/useEventBusForWeb";
import {
  push,
  popBackTo,
  pop,
  selectPages,
} from "../features/Pages/pagesSlice";
import { useEventBusForApp } from "../hooks/useEventBusForApp";
import { AbortControllerProvider } from "../contexts/AbortControllers";
import { Toolbar } from "../components/Toolbar";
import { Tab } from "../components/Toolbar/Toolbar";
import { PageWrapper } from "../components/PageWrapper";
import { ThreadHistory } from "./ThreadHistory";

import { LoginPage } from "./Login";
import { selectOpenTasksFromRoot, TaskList, TaskWorkspace } from "./Tasks";
import { KnowledgeWorkspace } from "./Knowledge";

import { StatsDashboard } from "./StatsDashboard";
import { RefactDaemonPage } from "./RefactDaemon";
import { Dashboard } from "./Dashboard";
import { SettingsHub, isSettingsPage } from "./Settings";
import { BuddyHome } from "./Buddy/BuddyHome";
import { BuddyErrorBoundary } from "./Buddy/BuddyErrorBoundary";

import { ChatLoading } from "../components/ChatContent/ChatLoading";
import { SplashScreen } from "./Splash";
import { selectBackendLastOkAt, selectBackendStatus } from "./Connection";
import {
  beginBuddyCrashSession,
  buildBuddyCrashRecoveryError,
  closeBuddyCrashSession,
  reportBuddyFrontendError,
  touchBuddyCrashSession,
} from "./Buddy/reportBuddyFrontendError";

import styles from "./App.module.css";
import { usePatchesAndDiffsEventsForIDE } from "../hooks/usePatchesAndDiffEventsForIDE";
import {
  getProjectStorageNamespace,
  isProjectStorageNamespaceTrusted,
  loadPersistedActiveTab,
  savePersistedActiveTab,
} from "../utils/chatUiPersistence";
import { selectFocusedWorkspaceChatId } from "./Workspace";
import { InternalLinkProvider } from "../contexts/InternalLinkContext";
import { parseRefactLink } from "../contexts/internalLinkUtils";
import { ProcessCompletedToasts } from "./Notifications";
import { hasUsableEngineEndpoint } from "../services/refact/apiUrl";
import { isPointerDragHost } from "./ChatPanes/pointerDrag";
import { PointerDragGhost } from "./ChatPanes/PointerDragGhost";

const STARTUP_SPLASH_DEADLINE_MS = 12_000;
const APP_ACCESS_LOSS_GRACE_MS = 3_000;

export interface AppProps {
  style?: React.CSSProperties;
}

export const InnerApp: React.FC<AppProps> = ({ style }: AppProps) => {
  const dispatch = useAppDispatch();
  const rootRef = useRef<HTMLDivElement>(null);
  const sawZeroHeightRef = useRef(false);
  const crashSessionStartedRef = useRef(false);
  const restoredActiveTabRef = useRef(false);
  const persistedActiveTabRef = useRef<ReturnType<
    typeof loadPersistedActiveTab
  > | null>(null);
  const lastProjectStorageRestoreIdentityRef = useRef<string | null>(null);
  const lastTrustedProjectStorageNamespaceRef = useRef<string | null>(null);

  const pages = useAppSelector(selectPages);
  const isStreaming = useAppSelector(selectIsStreaming);
  const allThreads = useAppSelector(selectAllThreads);
  const openTasks = useAppSelector(selectOpenTasksFromRoot);
  const focusedWorkspaceChatId = useAppSelector(selectFocusedWorkspaceChatId);

  const isPageInHistory = useCallback(
    (pageName: string) => {
      return pages.some((page) => page.name === pageName);
    },
    [pages],
  );

  const { chatPageChange, setIsChatStreaming, setIsChatReady } =
    useEventsBusForIDE();
  const chatId = useAppSelector(selectChatId);
  const currentThread = useAppSelector(selectThread);
  const backgroundAgents = useAppSelector((state) =>
    selectBackgroundAgentsByThread(state, chatId),
  );
  const backendStatus = useAppSelector(selectBackendStatus);
  const backendLastOkAt = useAppSelector(selectBackendLastOkAt);
  const providerBootstrap = useProviderBootstrapState();
  useEventBusForWeb();
  useEventBusForApp();
  usePatchesAndDiffsEventsForIDE();
  useSidebarSubscription();
  useAllChatsSubscription();
  useBrowserOnlineStatus();

  const config = useConfig();
  const projectStorageNamespaceTrusted = isProjectStorageNamespaceTrusted();
  const projectStorageNamespace = getProjectStorageNamespace();
  const trustedProjectStorageNamespace = projectStorageNamespaceTrusted
    ? projectStorageNamespace
    : null;
  const projectStorageRestoreIdentity = projectStorageNamespaceTrusted
    ? `trusted:${projectStorageNamespace ?? ""}`
    : "untrusted";

  useEffectOnce(() => {
    if (crashSessionStartedRef.current) return;
    crashSessionStartedRef.current = true;

    const previous = beginBuddyCrashSession({
      host: config.host,
      page: pages[pages.length - 1]?.name,
      chatId,
      isStreaming,
    });

    if (previous) {
      void reportBuddyFrontendError({
        source: "possible_renderer_crash",
        error: buildBuddyCrashRecoveryError(previous),
        sourceFile: "frontend/possible_renderer_crash",
        toolName: "renderer_crash_recovery",
        chatId: previous.chatId,
      });
    }

    const onPageHide = () => {
      closeBuddyCrashSession("pagehide");
    };

    window.addEventListener("pagehide", onPageHide);
    window.addEventListener("beforeunload", onPageHide);
    return () => {
      window.removeEventListener("pagehide", onPageHide);
      window.removeEventListener("beforeunload", onPageHide);
      closeBuddyCrashSession("unmount");
    };
  });

  useEffect(() => {
    touchBuddyCrashSession({
      host: config.host,
      page: pages[pages.length - 1]?.name,
      chatId,
      isStreaming,
    });
  }, [config.host, pages, chatId, isStreaming]);

  const checkIdeRootLayout = useCallback(() => {
    if (config.host !== "jetbrains" && config.host !== "ide") return;

    const elem = rootRef.current;
    if (!elem) return;

    const rect = elem.getBoundingClientRect();
    const height = Math.max(elem.clientHeight, rect.height);

    if (height <= 0) {
      sawZeroHeightRef.current = true;
      return;
    }

    if (!sawZeroHeightRef.current) return;

    sawZeroHeightRef.current = false;
    requestAnimationFrame(() => {
      window.dispatchEvent(new Event("resize"));
    });
  }, [config.host]);

  useResizeObserverOnRef(rootRef, checkIdeRootLayout);

  useEffect(() => {
    if (config.host !== "jetbrains" && config.host !== "ide") return;

    const onResize = () => {
      checkIdeRootLayout();
    };

    window.addEventListener("resize", onResize);
    const rafId = requestAnimationFrame(() => {
      checkIdeRootLayout();
    });

    return () => {
      window.removeEventListener("resize", onResize);
      cancelAnimationFrame(rafId);
    };
  }, [checkIdeRootLayout, config.host]);

  useEffect(() => {
    const onError = (event: ErrorEvent) => {
      void reportBuddyFrontendError({
        source: "window_error",
        error: event.error ?? event.message,
        sourceFile: event.filename || "frontend/window_error",
        chatId,
      });
    };

    const onRejection = (event: PromiseRejectionEvent) => {
      void reportBuddyFrontendError({
        source: "unhandledrejection",
        error: event.reason,
        sourceFile: "frontend/unhandledrejection",
        chatId,
      });
    };

    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, [chatId]);

  const desiredPage = pages[pages.length - 1];
  const [renderedPage, setRenderedPage] = useState(desiredPage);

  useEffect(() => {
    if (desiredPage === renderedPage) return;
    if (
      desiredPage.name === renderedPage.name &&
      desiredPage.name !== "task workspace" &&
      desiredPage.name !== "thread history page"
    ) {
      setRenderedPage(desiredPage);
      return;
    }
    const rafId = requestAnimationFrame(() => {
      setRenderedPage(desiredPage);
    });
    return () => cancelAnimationFrame(rafId);
  }, [desiredPage, renderedPage]);

  const pageSwitching = desiredPage !== renderedPage;

  const isLoggedIn =
    isPageInHistory("history") ||
    isPageInHistory("chat") ||
    isPageInHistory("tasks list") ||
    isPageInHistory("task workspace") ||
    isPageInHistory("task agent");

  const canAccessApp = providerBootstrap.canAccessApp;
  const hadAppAccessRef = useRef(canAccessApp);
  const [startupResolved, setStartupResolved] = useState(false);
  const [startupDeadlineReached, setStartupDeadlineReached] = useState(false);
  const hasEndpoint = hasUsableEngineEndpoint(config);
  const isBackendInstalling = providerBootstrap.status === "backend_installing";

  useEffect(() => {
    setStartupDeadlineReached(false);
    const timeoutId = setTimeout(() => {
      setStartupDeadlineReached(true);
    }, STARTUP_SPLASH_DEADLINE_MS);
    return () => clearTimeout(timeoutId);
  }, [
    config.dev,
    config.engineServed,
    config.host,
    config.lspPort,
    config.lspUrl,
  ]);

  useEffect(() => {
    if (
      backendStatus !== "online" ||
      providerBootstrap.status === "backend_connecting" ||
      providerBootstrap.status === "backend_installing"
    ) {
      setStartupResolved(false);
      return;
    }

    if (providerBootstrap.status !== "provider_loading") {
      setStartupResolved(true);
    }
  }, [backendStatus, providerBootstrap.status]);

  const showStartupSplash =
    !startupDeadlineReached &&
    (hasEndpoint || isBackendInstalling) &&
    !startupResolved &&
    backendLastOkAt === null &&
    providerBootstrap.status !== "ready" &&
    providerBootstrap.status !== "setup_required" &&
    providerBootstrap.status !== "provider_error";
  useEffect(() => {
    if (canAccessApp) {
      hadAppAccessRef.current = true;
      if (!isLoggedIn) {
        dispatch(push({ name: "history" }));
      }
      return;
    }

    const shouldShowLoginPage =
      providerBootstrap.status !== "backend_connecting" &&
      providerBootstrap.status !== "backend_installing" &&
      providerBootstrap.status !== "provider_loading";
    if (!shouldShowLoginPage || desiredPage.name === "login page") return;

    if (!hadAppAccessRef.current) {
      dispatch(popBackTo({ name: "login page" }));
      return;
    }

    const timeoutId = setTimeout(() => {
      dispatch(popBackTo({ name: "login page" }));
    }, APP_ACCESS_LOSS_GRACE_MS);

    return () => clearTimeout(timeoutId);
  }, [
    canAccessApp,
    providerBootstrap.status,
    desiredPage.name,
    isLoggedIn,
    dispatch,
  ]);

  useEffect(() => {
    if (pages.length > 1) {
      const currentPage = pages.slice(-1)[0];
      chatPageChange(currentPage.name);
    }
  }, [pages, chatPageChange]);

  useEffect(() => {
    setIsChatStreaming(isStreaming);
  }, [isStreaming, setIsChatStreaming]);

  useEffectOnce(() => {
    setIsChatReady(true);
  });

  const goBack = useCallback(() => {
    dispatch(pop());
  }, [dispatch]);

  const handleInternalLink = useCallback(
    (url: string): boolean => {
      const parsed = parseRefactLink(url);
      if (!parsed || parsed.type !== "chat" || !parsed.id) return false;

      const agent = Object.values(backgroundAgents).find(
        (candidate) => candidate.child_chat_id === parsed.id,
      );
      dispatch(
        createChatWithId({
          id: parsed.id,
          parentId: currentThread?.id,
          linkType: agent?.kind ?? currentThread?.link_type ?? "subagent",
        }),
      );
      dispatch(switchToThread({ id: parsed.id }));
      dispatch(popBackTo({ name: "history" }));
      dispatch(push({ name: "chat" }));
      return true;
    },
    [backgroundAgents, currentThread?.id, currentThread?.link_type, dispatch],
  );

  const pageWrapperStyle = useMemo<React.CSSProperties | undefined>(() => {
    if (renderedPage.name === "history") {
      return {
        paddingTop: 0,
        paddingRight: 0,
        paddingBottom: 0,
        paddingLeft: 0,
      };
    }

    return undefined;
  }, [renderedPage.name]);

  const activeTab: Tab | undefined = useMemo(() => {
    if (desiredPage.name === "chat") {
      return {
        type: "chat",
        id: focusedWorkspaceChatId ?? chatId,
      };
    }
    if (desiredPage.name === "history") {
      return {
        type: "dashboard",
      };
    }
    if (desiredPage.name === "task workspace") {
      return {
        type: "task",
        taskId: desiredPage.taskId,
        taskName: "",
      };
    }
    if (desiredPage.name === "buddy") {
      return {
        type: "buddy",
      };
    }
    if (
      desiredPage.name === "knowledge graph" ||
      desiredPage.name === "refact daemon"
    ) {
      return {
        type: "dashboard",
      };
    }
  }, [desiredPage, chatId, focusedWorkspaceChatId]);

  useEffect(() => {
    const previousIdentity = lastProjectStorageRestoreIdentityRef.current;
    if (previousIdentity === null) {
      lastProjectStorageRestoreIdentityRef.current =
        projectStorageRestoreIdentity;
      if (trustedProjectStorageNamespace !== null) {
        lastTrustedProjectStorageNamespaceRef.current =
          trustedProjectStorageNamespace;
      }
      return;
    }

    if (projectStorageRestoreIdentity !== previousIdentity) {
      const previousTrustedNamespace =
        lastTrustedProjectStorageNamespaceRef.current;
      if (
        trustedProjectStorageNamespace !== null &&
        previousTrustedNamespace !== null &&
        trustedProjectStorageNamespace !== previousTrustedNamespace
      ) {
        restoredActiveTabRef.current = false;
        persistedActiveTabRef.current = null;
      }
    }

    if (trustedProjectStorageNamespace !== null) {
      lastTrustedProjectStorageNamespaceRef.current =
        trustedProjectStorageNamespace;
    }
    lastProjectStorageRestoreIdentityRef.current =
      projectStorageRestoreIdentity;
  }, [projectStorageRestoreIdentity, trustedProjectStorageNamespace]);

  useEffect(() => {
    if (!restoredActiveTabRef.current) return;
    if (!activeTab) return;
    if (activeTab.type === "chat" && !activeTab.id) return;

    if (activeTab.type === "task") {
      savePersistedActiveTab({ type: "task", taskId: activeTab.taskId });
      return;
    }

    if (activeTab.type === "buddy") {
      savePersistedActiveTab({ type: "buddy" });
      return;
    }

    savePersistedActiveTab(activeTab);
  }, [activeTab]);

  useEffect(() => {
    if (restoredActiveTabRef.current) return;
    if (!canAccessApp || !isLoggedIn) return;
    if (!isProjectStorageNamespaceTrusted()) return;

    const persistedActiveTab =
      persistedActiveTabRef.current ?? loadPersistedActiveTab();
    persistedActiveTabRef.current = persistedActiveTab;
    if (!persistedActiveTab) {
      restoredActiveTabRef.current = true;
      return;
    }

    if (persistedActiveTab.type === "dashboard") {
      restoredActiveTabRef.current = true;
      dispatch(popBackTo({ name: "history" }));
      return;
    }

    if (persistedActiveTab.type === "buddy") {
      restoredActiveTabRef.current = true;
      dispatch(popBackTo({ name: "history" }));
      dispatch(push({ name: "buddy" }));
      return;
    }

    if (persistedActiveTab.type === "chat") {
      const restoredChatId =
        focusedWorkspaceChatId && allThreads[focusedWorkspaceChatId]
          ? focusedWorkspaceChatId
          : persistedActiveTab.id;
      restoredActiveTabRef.current = true;
      if (allThreads[restoredChatId]) {
        dispatch(switchToThread({ id: restoredChatId, openTab: false }));
        dispatch(popBackTo({ name: "history" }));
        dispatch(push({ name: "chat" }));
      } else {
        dispatch(popBackTo({ name: "history" }));
      }
      return;
    }

    restoredActiveTabRef.current = true;
    dispatch(popBackTo({ name: "history" }));
    if (openTasks.some((task) => task.id === persistedActiveTab.taskId)) {
      dispatch(
        push({ name: "task workspace", taskId: persistedActiveTab.taskId }),
      );
    }
  }, [
    allThreads,
    canAccessApp,
    dispatch,
    focusedWorkspaceChatId,
    isLoggedIn,
    openTasks,
  ]);

  const startupSplashMessage =
    providerBootstrap.status === "backend_installing"
      ? "Downloading Refact engine…"
      : backendStatus === "online"
        ? "Loading your providers…"
        : "Starting local Refact engine…";

  return (
    <Flex
      ref={rootRef}
      align="stretch"
      direction="column"
      style={style}
      className={styles.rootFlex}
      data-element="app-root"
    >
      {showStartupSplash ? (
        <SplashScreen message={startupSplashMessage} />
      ) : (
        <>
          {activeTab && <Toolbar activeTab={activeTab} />}
          <PageWrapper host={config.host} style={pageWrapperStyle}>
            {renderedPage.name === "login page" && <LoginPage />}
            {pageSwitching && <ChatLoading />}
            {!pageSwitching && renderedPage.name === "history" && <Dashboard />}
            {!pageSwitching && renderedPage.name === "chat" && (
              <InternalLinkProvider onInternalLink={handleInternalLink}>
                <WorkspaceView />
              </InternalLinkProvider>
            )}
            {!pageSwitching && isSettingsPage(renderedPage) && (
              <SettingsHub
                page={renderedPage}
                onBack={goBack}
                host={config.host}
                tabbed={config.tabbed}
              />
            )}
            {!pageSwitching && renderedPage.name === "thread history page" && (
              <ThreadHistory
                backFromThreadHistory={goBack}
                tabbed={config.tabbed}
                host={config.host}
                onCloseThreadHistory={goBack}
                chatId={renderedPage.chatId}
              />
            )}
            {!pageSwitching && renderedPage.name === "tasks list" && (
              <TaskList backFromTasks={goBack} />
            )}
            {!pageSwitching && renderedPage.name === "task workspace" && (
              <TaskWorkspace
                key={renderedPage.taskId}
                taskId={renderedPage.taskId}
              />
            )}
            {!pageSwitching && renderedPage.name === "knowledge graph" && (
              <KnowledgeWorkspace />
            )}

            {!pageSwitching && renderedPage.name === "stats dashboard" && (
              <StatsDashboard
                backFromDashboard={goBack}
                tabbed={config.tabbed}
                host={config.host}
              />
            )}

            {!pageSwitching && renderedPage.name === "refact daemon" && (
              <RefactDaemonPage backFromDaemon={goBack} />
            )}

            {!pageSwitching && renderedPage.name === "buddy" && <BuddyHome />}
          </PageWrapper>
          <ProcessCompletedToasts />
          {isPointerDragHost(config.host) && <PointerDragGhost />}
        </>
      )}
    </Flex>
  );
};

// TODO: move this to the `app` directory.
export const App = () => {
  return (
    <BuddyErrorBoundary>
      <Provider store={store}>
        <Theme>
          <AbortControllerProvider>
            <BuddyErrorBoundary showThreadReportPanel>
              <InnerApp />
            </BuddyErrorBoundary>
          </AbortControllerProvider>
        </Theme>
      </Provider>
    </BuddyErrorBoundary>
  );
};
