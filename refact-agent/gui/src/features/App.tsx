import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Flex } from "@radix-ui/themes";
import {
  Chat,
  createChatWithId,
  selectAllThreads,
  selectBackgroundAgentsByThread,
  selectChatId,
  selectIsStreaming,
  selectThread,
  switchToThread,
} from "./Chat";

import {
  useAppSelector,
  useAppDispatch,
  useConfig,
  useEffectOnce,
  useEventsBusForIDE,
  useSidebarSubscription,
  useAllChatsSubscription,
  useGetConfiguredProvidersQuery,
  useResizeObserverOnRef,
} from "../hooks";
import { useGetPing } from "../hooks/useGetPing";
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
import { Integrations } from "./Integrations";
import { Providers } from "./Providers";
import { integrationsApi } from "../services/refact";
import { LoginPage } from "./Login";
import { selectOpenTasksFromRoot, TaskList, TaskWorkspace } from "./Tasks";
import { KnowledgeWorkspace } from "./Knowledge";
import { Customization } from "./Customization";
import { Extensions } from "./Extensions";
import { DefaultModels } from "./DefaultModels";
import { MCPMarketplace } from "./MCPMarketplace";
import { SkillsMarketplace } from "./SkillsMarketplace";
import { CommandsMarketplace } from "./CommandsMarketplace";
import { SubagentsMarketplace } from "./SubagentsMarketplace";
import { MarketplaceHub } from "./MarketplaceHub";
import { StatsDashboard } from "./StatsDashboard";
import { Dashboard } from "./Dashboard";
import { BuddyHome } from "./Buddy/BuddyHome";
import { BuddyConductorGoalsPanel } from "./Buddy/BuddyConductorGoalsPanel";
import { BuddyErrorBoundary } from "./Buddy/BuddyErrorBoundary";
import { SchedulerPanel } from "./Scheduler";
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
import classNames from "classnames";
import { usePatchesAndDiffsEventsForIDE } from "../hooks/usePatchesAndDiffEventsForIDE";
import { hasAnyUsableActiveProvider } from "./Login/providerAccess";
import {
  isProjectStorageNamespaceTrusted,
  loadPersistedActiveTab,
  savePersistedActiveTab,
} from "../utils/chatUiPersistence";
import { InternalLinkProvider } from "../contexts/InternalLinkContext";
import { parseRefactLink } from "../contexts/internalLinkUtils";
import { ProcessCompletedToasts } from "./Notifications";
import { hasUsableEngineEndpoint } from "../services/refact/apiUrl";

const STARTUP_SPLASH_DEADLINE_MS = 12_000;

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

  const pages = useAppSelector(selectPages);
  const isStreaming = useAppSelector(selectIsStreaming);
  const allThreads = useAppSelector(selectAllThreads);
  const openTasks = useAppSelector(selectOpenTasksFromRoot);

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
  const providersQuery = useGetConfiguredProvidersQuery();
  useEventBusForWeb();
  useEventBusForApp();
  usePatchesAndDiffsEventsForIDE();
  useSidebarSubscription();
  useAllChatsSubscription();
  useGetPing();
  useBrowserOnlineStatus();

  const [isPaddingApplied, setIsPaddingApplied] = useState<boolean>(false);

  const handlePaddingShift = useCallback((state: boolean) => {
    setIsPaddingApplied(state);
  }, []);

  const config = useConfig();

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

  const isLoggedIn = isPageInHistory("history") || isPageInHistory("chat");

  const hasAnyActiveProvider = useMemo(() => {
    return hasAnyUsableActiveProvider({
      providers: providersQuery.data?.providers ?? [],
    });
  }, [providersQuery.data?.providers]);
  const canAccessApp = hasAnyActiveProvider;
  const canResolveProviderAccess = providersQuery.isSuccess;
  const [startupResolved, setStartupResolved] = useState(false);
  const [startupDeadlineReached, setStartupDeadlineReached] = useState(false);
  const hasEndpoint = hasUsableEngineEndpoint(config);

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
    if (backendStatus !== "online") {
      setStartupResolved(false);
      return;
    }

    if (providersQuery.isSuccess || providersQuery.isError) {
      setStartupResolved(true);
    }
  }, [backendStatus, providersQuery.isError, providersQuery.isSuccess]);

  const showStartupSplash =
    !startupDeadlineReached &&
    hasEndpoint &&
    !startupResolved &&
    backendLastOkAt === null &&
    (backendStatus !== "online" ||
      providersQuery.isUninitialized ||
      providersQuery.isLoading ||
      providersQuery.isFetching);

  useEffect(() => {
    if (canAccessApp && !isLoggedIn) {
      dispatch(push({ name: "history" }));
    }

    if (
      !canAccessApp &&
      canResolveProviderAccess &&
      desiredPage.name !== "login page"
    ) {
      dispatch(popBackTo({ name: "login page" }));
    }
  }, [
    canAccessApp,
    canResolveProviderAccess,
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

  const goBackFromIntegrations = useCallback(() => {
    dispatch(pop());
    dispatch(integrationsApi.util.resetApiState());
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

    if (renderedPage.name === "integrations page") {
      return { paddingRight: 0 };
    }

    return undefined;
  }, [renderedPage.name]);

  const activeTab: Tab | undefined = useMemo(() => {
    if (desiredPage.name === "chat") {
      return {
        type: "chat",
        id: chatId,
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
    if (desiredPage.name === "knowledge graph") {
      return {
        type: "dashboard",
      };
    }
  }, [desiredPage, chatId]);

  useEffect(() => {
    if (!restoredActiveTabRef.current) return;
    if (!activeTab) return;
    if (activeTab.type === "chat" && !activeTab.id) return;

    if (activeTab.type === "task") {
      savePersistedActiveTab({ type: "task", taskId: activeTab.taskId });
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

    if (persistedActiveTab.type === "chat") {
      if (!allThreads[persistedActiveTab.id]) return;
      restoredActiveTabRef.current = true;
      dispatch(switchToThread({ id: persistedActiveTab.id }));
      dispatch(popBackTo({ name: "history" }));
      dispatch(push({ name: "chat" }));
      return;
    }

    if (openTasks.some((task) => task.id === persistedActiveTab.taskId)) {
      restoredActiveTabRef.current = true;
      dispatch(popBackTo({ name: "history" }));
      dispatch(
        push({ name: "task workspace", taskId: persistedActiveTab.taskId }),
      );
    }
  }, [allThreads, canAccessApp, dispatch, isLoggedIn, openTasks]);

  return (
    <Flex
      ref={rootRef}
      align="stretch"
      direction="column"
      style={style}
      className={classNames(styles.rootFlex, {
        [styles.integrationsPagePadding]:
          renderedPage.name === "integrations page" && isPaddingApplied,
      })}
      data-element="app-root"
    >
      {showStartupSplash ? (
        <SplashScreen
          message={
            backendStatus === "online"
              ? "Loading your providers…"
              : "Starting local Refact engine…"
          }
        />
      ) : (
        <>
          {activeTab && <Toolbar activeTab={activeTab} />}
          <PageWrapper host={config.host} style={pageWrapperStyle}>
            {renderedPage.name === "login page" && <LoginPage />}
            {pageSwitching && <ChatLoading />}
            {!pageSwitching && renderedPage.name === "history" && <Dashboard />}
            {!pageSwitching && renderedPage.name === "chat" && (
              <InternalLinkProvider onInternalLink={handleInternalLink}>
                <Chat
                  host={config.host}
                  tabbed={config.tabbed}
                  backFromChat={goBack}
                />
              </InternalLinkProvider>
            )}
            {!pageSwitching && renderedPage.name === "integrations page" && (
              <Integrations
                backFromIntegrations={goBackFromIntegrations}
                tabbed={config.tabbed}
                host={config.host}
                onCloseIntegrations={goBackFromIntegrations}
                handlePaddingShift={handlePaddingShift}
              />
            )}
            {!pageSwitching && renderedPage.name === "providers page" && (
              <Providers
                backFromProviders={goBack}
                tabbed={config.tabbed}
                host={config.host}
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
            {!pageSwitching && renderedPage.name === "customization" && (
              <Customization
                backFromCustomization={goBack}
                tabbed={config.tabbed}
                host={config.host}
                initialKind={renderedPage.kind}
                initialConfigId={renderedPage.configId}
                draftId={renderedPage.draftId}
              />
            )}
            {!pageSwitching && renderedPage.name === "default models" && (
              <DefaultModels
                backFromDefaultModels={goBack}
                tabbed={config.tabbed}
                host={config.host}
                draftId={renderedPage.draftId}
              />
            )}
            {!pageSwitching && renderedPage.name === "stats dashboard" && (
              <StatsDashboard
                backFromDashboard={goBack}
                tabbed={config.tabbed}
                host={config.host}
              />
            )}
            {!pageSwitching && renderedPage.name === "extensions" && (
              <Extensions
                backFromExtensions={goBack}
                tabbed={config.tabbed}
                host={config.host}
                initialTab={renderedPage.tab}
                initialItemId={renderedPage.itemId}
                draftId={renderedPage.draftId}
              />
            )}
            {!pageSwitching && renderedPage.name === "mcp marketplace" && (
              <MCPMarketplace
                backFromMarketplace={goBack}
                tabbed={config.tabbed}
                host={config.host}
              />
            )}
            {!pageSwitching && renderedPage.name === "skills marketplace" && (
              <SkillsMarketplace
                backFromMarketplace={goBack}
                tabbed={config.tabbed}
                host={config.host}
              />
            )}
            {!pageSwitching && renderedPage.name === "commands marketplace" && (
              <CommandsMarketplace
                backFromMarketplace={goBack}
                tabbed={config.tabbed}
                host={config.host}
              />
            )}
            {!pageSwitching &&
              renderedPage.name === "subagents marketplace" && (
                <SubagentsMarketplace
                  backFromMarketplace={goBack}
                  tabbed={config.tabbed}
                  host={config.host}
                />
              )}
            {!pageSwitching && renderedPage.name === "marketplace hub" && (
              <MarketplaceHub
                back={goBack}
                tabbed={config.tabbed}
                host={config.host}
              />
            )}
            {!pageSwitching && renderedPage.name === "buddy" && <BuddyHome />}
            {!pageSwitching && renderedPage.name === "conductor" && (
              <BuddyConductorGoalsPanel />
            )}
            {!pageSwitching && renderedPage.name === "scheduler" && (
              <SchedulerPanel onBack={goBack} />
            )}
          </PageWrapper>
          <ProcessCompletedToasts />
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
