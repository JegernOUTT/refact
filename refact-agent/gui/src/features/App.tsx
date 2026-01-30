import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Flex } from "@radix-ui/themes";
import { Chat, newChatAction, selectChatId, selectIsStreaming } from "./Chat";
import { Sidebar } from "../components/Sidebar/Sidebar";
import {
  useAppSelector,
  useAppDispatch,
  useConfig,
  useEffectOnce,
  useEventsBusForIDE,
  useSidebarSubscription,
  useAllChatsSubscription,
} from "../hooks";
import { useBrowserOnlineStatus } from "../hooks/useBrowserOnlineStatus";
import { FIMDebug } from "./FIM";
import { store, persistor, RootState } from "../app/store";
import { Provider } from "react-redux";
import { PersistGate } from "redux-persist/integration/react";
import { Theme } from "../components/Theme";
import { useEventBusForWeb } from "../hooks/useEventBusForWeb";
import { Statistics } from "./Statistics";
import { Welcome } from "../components/Tour";
import {
  push,
  popBackTo,
  pop,
  selectPages,
} from "../features/Pages/pagesSlice";
import { TourProvider } from "./Tour";
import { Tour } from "../components/Tour";
import { TourEnd } from "../components/Tour/TourEnd";
import { useEventBusForApp } from "../hooks/useEventBusForApp";
import { AbortControllerProvider } from "../contexts/AbortControllers";
import { Toolbar } from "../components/Toolbar";
import { Tab } from "../components/Toolbar/Toolbar";
import { PageWrapper } from "../components/PageWrapper";
import { ThreadHistory } from "./ThreadHistory";
import { Integrations } from "./Integrations";
import { Providers } from "./Providers";
import { UserSurvey } from "./UserSurvey";
import { integrationsApi } from "../services/refact";
import { LoginPage } from "./Login";
import { TaskList, TaskWorkspace } from "./Tasks";
import { KnowledgeWorkspace } from "./Knowledge";
import { Customization } from "./Customization";
import { ChatLoading } from "../components/ChatContent/ChatLoading";

import styles from "./App.module.css";
import classNames from "classnames";
import { usePatchesAndDiffsEventsForIDE } from "../hooks/usePatchesAndDiffEventsForIDE";
import { UrqlProvider } from "../../urqlProvider";
import { selectActiveGroup } from "./Teams";

export interface AppProps {
  style?: React.CSSProperties;
}

export const InnerApp: React.FC<AppProps> = ({ style }: AppProps) => {
  const dispatch = useAppDispatch();

  const pages = useAppSelector(selectPages);
  const isStreaming = useAppSelector(selectIsStreaming);

  const isPageInHistory = useCallback(
    (pageName: string) => {
      return pages.some((page) => page.name === pageName);
    },
    [pages],
  );

  const { chatPageChange, setIsChatStreaming, setIsChatReady } =
    useEventsBusForIDE();
  const tourState = useAppSelector((state: RootState) => state.tour);
  const historyState = useAppSelector((state: RootState) => state.history);
  const maybeCurrentActiveGroup = useAppSelector(selectActiveGroup);
  const chatId = useAppSelector(selectChatId);
  useEventBusForWeb();
  useEventBusForApp();
  usePatchesAndDiffsEventsForIDE();
  useSidebarSubscription();
  useAllChatsSubscription();
  useBrowserOnlineStatus();

  const [isPaddingApplied, setIsPaddingApplied] = useState<boolean>(false);

  const handlePaddingShift = (state: boolean) => {
    setIsPaddingApplied(state);
  };

  const config = useConfig();

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
    isPageInHistory("welcome") ||
    isPageInHistory("chat");

  useEffect(() => {
    if (config.apiKey && config.addressURL && !isLoggedIn) {
      if (tourState.type === "in_progress" && tourState.step === 1) {
        dispatch(push({ name: "welcome" }));
      } else if (
        !historyState.isLoading &&
        Object.keys(historyState.chats).length === 0 &&
        maybeCurrentActiveGroup
      ) {
        dispatch(push({ name: "history" }));
        dispatch(newChatAction());
        dispatch(push({ name: "chat" }));
      } else {
        dispatch(push({ name: "history" }));
      }
    }
    if (!config.apiKey && !config.addressURL && isLoggedIn) {
      dispatch(popBackTo({ name: "login page" }));
    }
  }, [
    config.apiKey,
    config.addressURL,
    isLoggedIn,
    dispatch,
    tourState,
    historyState,
    maybeCurrentActiveGroup,
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

  const startTour = () => {
    dispatch(push({ name: "history" }));
  };

  const goBack = () => {
    dispatch(pop());
  };

  const goBackFromIntegrations = () => {
    dispatch(pop());
    dispatch(integrationsApi.util.resetApiState());
  };

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

  return (
    <Flex
      align="stretch"
      direction="column"
      style={style}
      className={classNames(styles.rootFlex, {
        [styles.integrationsPagePadding]:
          renderedPage.name === "integrations page" && isPaddingApplied,
      })}
    >
      {activeTab && <Toolbar activeTab={activeTab} />}
      <PageWrapper
        host={config.host}
        style={{
          paddingRight:
            renderedPage.name === "integrations page" ? 0 : undefined,
        }}
      >
        <UserSurvey />
        {renderedPage.name === "login page" && <LoginPage />}
        {pageSwitching && <ChatLoading />}
        {!pageSwitching && renderedPage.name === "welcome" && (
          <Welcome onPressNext={startTour} />
        )}
        {!pageSwitching && renderedPage.name === "tour end" && <TourEnd />}
        {!pageSwitching && renderedPage.name === "history" && (
          <Sidebar
            takingNotes={false}
            onOpenChatInTab={undefined}
            style={{
              alignSelf: "stretch",
              height: "calc(100% - var(--space-5)* 2)",
            }}
          />
        )}
        {!pageSwitching && renderedPage.name === "chat" && (
          <Chat
            host={config.host}
            tabbed={config.tabbed}
            backFromChat={goBack}
          />
        )}
        {!pageSwitching &&
          renderedPage.name === "fill in the middle debug page" && (
            <FIMDebug host={config.host} tabbed={config.tabbed} />
          )}
        {!pageSwitching && renderedPage.name === "statistics page" && (
          <Statistics
            backFromStatistic={goBack}
            tabbed={config.tabbed}
            host={config.host}
            onCloseStatistic={goBack}
          />
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
        {!pageSwitching && renderedPage.name === "tasks list" && <TaskList />}
        {!pageSwitching && renderedPage.name === "task workspace" && (
          <TaskWorkspace taskId={renderedPage.taskId} />
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
          />
        )}
      </PageWrapper>
      {renderedPage.name !== "welcome" && (
        <Tour page={pages[pages.length - 1].name} />
      )}
    </Flex>
  );
};

// TODO: move this to the `app` directory.
export const App = () => {
  return (
    <Provider store={store}>
      <UrqlProvider>
        <PersistGate persistor={persistor}>
          <Theme>
            <TourProvider>
              <AbortControllerProvider>
                <InnerApp />
              </AbortControllerProvider>
            </TourProvider>
          </Theme>
        </PersistGate>
      </UrqlProvider>
    </Provider>
  );
};
