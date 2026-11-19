import { useCallback, useState } from "react";
import { useStore } from "react-redux";
import type { AppStore } from "../../app/store";
import { useAppDispatch } from "../../hooks";
import {
  browserApi,
  isProjectBrowserTabGone,
  projectBrowserErrorMessage,
  type ProjectBrowserStatus,
} from "../../services/refact/browser";
import {
  isValidBrowserTabId,
  makeBrowserTabSurfaceKey,
} from "../Workspace/surfaceKey";
import { closeTab } from "../Workspace/workspaceSlice";
import { mostRecentChatTab } from "./browserTabTitle";
import {
  browserShowFinished,
  browserShowStarted,
  fetchBrowserStatusAfterNow,
  isBrowserViewUsable,
  openBrowserTabForChat,
  selectBrowserShowInProgress,
  selectChatBrowserViewTabIds,
  selectProjectBrowserStatus,
  selectUsableBrowserTabIds,
  storeStatusWaiter,
} from "./browserViews";

export const NEW_TAB_MISSING_ID =
  "The browser did not report the new tab, so it could not be shown.";
export const STATUS_REFRESH_FAILED =
  "Could not check the browser status. Try again.";

const useAppStore = useStore.withTypes<AppStore>();

export function useProjectBrowserTabActions() {
  const dispatch = useAppDispatch();
  const store = useAppStore();
  const [start] = browserApi.useProjectBrowserStartMutation();
  const [changeTab] = browserApi.useProjectBrowserTabMutation();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  const run = useCallback(async (work: () => Promise<string | undefined>) => {
    setPending(true);
    setError(undefined);
    try {
      setError(await work());
    } catch (cause) {
      setError(projectBrowserErrorMessage(cause));
    } finally {
      setPending(false);
    }
  }, []);

  const createTab = useCallback(
    async (chatId: string, status: ProjectBrowserStatus | undefined) => {
      if (!status?.running) await start(undefined).unwrap();
      const next = await changeTab({ op: "new", chat_id: chatId }).unwrap();
      if (!next.new_tab_id) return NEW_TAB_MISSING_ID;
      dispatch(openBrowserTabForChat({ chatId, tabId: next.new_tab_id }));
      return undefined;
    },
    [changeTab, dispatch, start],
  );

  const refreshStatus = useCallback(
    () =>
      dispatch(
        browserApi.endpoints.projectBrowserStatus.initiate(undefined, {
          forceRefetch: true,
          subscribe: false,
        }),
      ),
    [dispatch],
  );

  const fetchFreshStatus = useCallback(async () => {
    const fresh = await fetchBrowserStatusAfterNow(
      dispatch,
      () => store.getState(),
      storeStatusWaiter(store),
    );
    if (!fresh.fetched) return STATUS_REFRESH_FAILED;
    return fresh.error ? projectBrowserErrorMessage(fresh.error) : undefined;
  }, [dispatch, store]);

  const createTabForChat = useCallback(
    (chatId: string, status: ProjectBrowserStatus | undefined) =>
      run(() => createTab(chatId, status)),
    [createTab, run],
  );

  const showFresh = useCallback(
    async (chatId: string) => {
      const stale = await fetchFreshStatus();
      if (stale) return stale;
      const state = store.getState();
      const status = selectProjectBrowserStatus(state);
      const usable = selectUsableBrowserTabIds(state);
      const dead = selectChatBrowserViewTabIds(state, chatId).filter(
        (tabId) =>
          isValidBrowserTabId(tabId) &&
          !isBrowserViewUsable(state, usable, tabId),
      );
      const recent = status?.running
        ? mostRecentChatTab(status.tabs, chatId)
        : undefined;
      if (recent)
        dispatch(openBrowserTabForChat({ chatId, tabId: recent.tab_id }));
      else {
        const failure = await createTab(chatId, status);
        if (failure) return failure;
      }
      for (const tabId of dead)
        dispatch(closeTab(makeBrowserTabSurfaceKey(tabId)));
      return undefined;
    },
    [createTab, dispatch, fetchFreshStatus, store],
  );

  const showBrowserForChat = useCallback(
    async (chatId: string) => {
      if (selectBrowserShowInProgress(store.getState(), chatId)) return;
      dispatch(browserShowStarted(chatId));
      await run(async () => {
        try {
          return await showFresh(chatId);
        } finally {
          dispatch(browserShowFinished(chatId));
        }
      });
    },
    [dispatch, run, showFresh, store],
  );

  const closeBrowserTab = useCallback(
    (tabId: string) =>
      run(async () => {
        try {
          await changeTab({ op: "close", tab_id: tabId }).unwrap();
        } catch (cause) {
          if (!isProjectBrowserTabGone(cause)) throw cause;
          void refreshStatus();
        }
        if (isValidBrowserTabId(tabId))
          dispatch(closeTab(makeBrowserTabSurfaceKey(tabId)));
        return undefined;
      }),
    [changeTab, dispatch, refreshStatus, run],
  );

  return {
    createTabForChat,
    showBrowserForChat,
    closeBrowserTab,
    pending,
    error,
  };
}
