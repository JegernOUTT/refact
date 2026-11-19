import {
  createAction,
  createSelector,
  createSlice,
  isAnyOf,
  type Middleware,
  type PayloadAction,
} from "@reduxjs/toolkit";
import { QueryStatus } from "@reduxjs/toolkit/query";
import type { AppDispatch, AppStore, RootState } from "../../app/store";
import {
  browserApi,
  projectBrowserStatusPublishing,
  type ProjectBrowserStatus,
} from "../../services/refact/browser";
import { setInformation } from "../Errors/informationSlice";
import { isValidBrowserTabId } from "../Workspace/surfaceKey";
import {
  canShowBrowserTabForChat,
  isBrowserViewHeldByVisibleChat,
  pruneBrowserTabSurfaces,
  selectBrowserViewOwners,
  selectFocusedWorkspaceChatId,
  showBrowserTabForChat,
} from "../Workspace/workspaceSlice";

export const openBrowserTabForChat = createAction<{
  chatId?: string;
  tabId: string;
  automatic?: boolean;
}>("browserViews/openBrowserTabForChat");

const MAX_HANDLED_HINTS = 256;
const MAX_HINT_CHATS = 64;
const STALE_STATUS_MISSES = 2;

export const BROWSER_VIEW_CAP_NOTICE =
  "The browser tab could not be shown next to this chat because the workspace has too many panes or tabs. Close some and try again.";
export const BROWSER_VIEW_INVALID_TAB_NOTICE =
  "The browser reported a tab that cannot be shown in the workspace.";
export const BROWSER_VIEW_NO_CHAT_NOTICE =
  "Open a chat to show browser tabs next to it.";

export type BrowserViewsState = {
  names: Record<string, string>;
  handledHints: Partial<Record<string, string[]>>;
  stableStatus?: ProjectBrowserStatus | null;
  pendingStatusRequests: string[];
  awaitingTabs: Partial<Record<string, string[]>>;
  publishingRequest?: string | null;
  publishedBy: Partial<Record<string, string>>;
  showingChats: string[];
};

const initialState: BrowserViewsState = {
  names: {},
  handledHints: {},
  pendingStatusRequests: [],
  awaitingTabs: {},
  publishedBy: {},
  showingChats: [],
};

const isStatusMutationPending = isAnyOf(
  browserApi.endpoints.projectBrowserStart.matchPending,
  browserApi.endpoints.projectBrowserTab.matchPending,
  browserApi.endpoints.projectBrowserNavigate.matchPending,
);

const isStatusMutationSettled = isAnyOf(
  browserApi.endpoints.projectBrowserStart.matchFulfilled,
  browserApi.endpoints.projectBrowserStart.matchRejected,
  browserApi.endpoints.projectBrowserTab.matchFulfilled,
  browserApi.endpoints.projectBrowserTab.matchRejected,
  browserApi.endpoints.projectBrowserNavigate.matchFulfilled,
  browserApi.endpoints.projectBrowserNavigate.matchRejected,
);

const selectStatusResult =
  browserApi.endpoints.projectBrowserStatus.select(undefined);

const selectStatusData = (state: RootState) => selectStatusResult(state).data;

export const browserViewsSlice = createSlice({
  name: "browserViews",
  initialState,
  reducers: {
    browserShowStarted: (state, action: PayloadAction<string>) => {
      state.showingChats.push(action.payload);
    },
    browserShowFinished: (state, action: PayloadAction<string>) => {
      const index = state.showingChats.indexOf(action.payload);
      if (index >= 0) state.showingChats.splice(index, 1);
    },
    browserHintsHandled: (
      state,
      action: PayloadAction<{
        chatId: string;
        toolCallIds: string[];
        names?: Record<string, string>;
        hintedTabId?: string;
      }>,
    ) => {
      const { chatId, toolCallIds, names, hintedTabId } = action.payload;
      if (names) Object.assign(state.names, names);
      if (hintedTabId)
        state.awaitingTabs[hintedTabId] = [...state.pendingStatusRequests];
      if (toolCallIds.length === 0) return;
      const handled = state.handledHints[chatId] ?? [];
      const known = new Set(handled);
      const next = [
        ...handled,
        ...toolCallIds.filter((id) => {
          if (known.has(id)) return false;
          known.add(id);
          return true;
        }),
      ];
      state.handledHints = Object.fromEntries(
        [
          ...Object.entries(state.handledHints).filter(
            ([chat]) => chat !== chatId,
          ),
          [chatId, next.slice(-MAX_HANDLED_HINTS)] as const,
        ].slice(-MAX_HINT_CHATS),
      );
    },
  },
  extraReducers: (builder) => {
    const settle = (state: BrowserViewsState, requestId: string) => {
      state.pendingStatusRequests = state.pendingStatusRequests.filter(
        (id) => id !== requestId,
      );
      const origin = state.publishedBy[requestId];
      if (origin)
        state.publishedBy = Object.fromEntries(
          Object.entries(state.publishedBy).filter(([id]) => id !== requestId),
        );
      return origin ?? requestId;
    };
    builder
      .addCase(projectBrowserStatusPublishing, (state, action) => {
        state.publishingRequest = action.payload;
      })
      .addMatcher(
        browserApi.endpoints.projectBrowserStatus.matchPending,
        (state, action) => {
          if (state.publishingRequest) {
            state.publishedBy[action.meta.requestId] = state.publishingRequest;
            state.publishingRequest = null;
            return;
          }
          state.pendingStatusRequests.push(action.meta.requestId);
        },
      )
      .addMatcher(isStatusMutationPending, (state, action) => {
        state.pendingStatusRequests.push(action.meta.requestId);
      })
      .addMatcher(isStatusMutationSettled, (state, action) => {
        settle(state, action.meta.requestId);
      })
      .addMatcher(
        browserApi.endpoints.projectBrowserStatus.matchRejected,
        (state, action) => {
          settle(state, action.meta.requestId);
        },
      )
      .addMatcher(
        browserApi.endpoints.projectBrowserStatus.matchFulfilled,
        (state, action) => {
          const origin = settle(state, action.meta.requestId);
          if (action.payload.busy) return;
          state.stableStatus = action.payload;
          state.awaitingTabs = Object.fromEntries(
            Object.entries(state.awaitingTabs).filter(
              ([, stale]) => stale?.includes(origin),
            ),
          );
        },
      );
  },
  selectors: {
    selectBrowserTabHintNames: (state) => state.names,
    selectBrowserHintHandled: (state, chatId: string, toolCallId: string) =>
      state.handledHints[chatId]?.includes(toolCallId) ?? false,
    selectStableBrowserStatus: (state) => state.stableStatus ?? null,
    selectBrowserTabAwaitingStatus: (state, tabId: string) =>
      tabId in state.awaitingTabs,
    selectBrowserShowInProgress: (state, chatId: string) =>
      state.showingChats.includes(chatId),
  },
});

export const { browserHintsHandled, browserShowStarted, browserShowFinished } =
  browserViewsSlice.actions;
export const {
  selectBrowserTabHintNames,
  selectBrowserHintHandled,
  selectStableBrowserStatus,
  selectBrowserTabAwaitingStatus,
  selectBrowserShowInProgress,
} = browserViewsSlice.selectors;

export function withStableBrowserTabs(
  status: ProjectBrowserStatus | undefined,
  stable: ProjectBrowserStatus | null,
): ProjectBrowserStatus | undefined {
  if (!status?.busy || !stable) return status;
  if (status.runtime_id && stable.runtime_id !== status.runtime_id)
    return status;
  return {
    ...status,
    runtime_id: status.runtime_id ?? stable.runtime_id,
    active_tab: status.active_tab ?? stable.active_tab,
    tabs: stable.tabs,
  };
}

export const selectProjectBrowserStatus = createSelector(
  [selectStatusData, selectStableBrowserStatus],
  withStableBrowserTabs,
);

const BROWSER_STATUS_WAIT_MS = 10_000;
const MAX_FRESH_STATUS_ATTEMPTS = 5;

export type BrowserStatusWaiter = (
  done: (state: RootState) => boolean,
  timeoutMs: number,
) => Promise<boolean>;

export type FreshBrowserStatus =
  | { fetched: true; error?: unknown }
  | { fetched: false };

const isStatusPending = (state: RootState) =>
  selectStatusResult(state).status === QueryStatus.pending;

export const storeStatusWaiter =
  (store: AppStore): BrowserStatusWaiter =>
  (done, timeoutMs) =>
    new Promise((resolve) => {
      if (done(store.getState())) {
        resolve(true);
        return;
      }
      let unsubscribe: () => void = () => undefined;
      const timer = window.setTimeout(() => {
        unsubscribe();
        resolve(false);
      }, timeoutMs);
      unsubscribe = store.subscribe(() => {
        if (!done(store.getState())) return;
        window.clearTimeout(timer);
        unsubscribe();
        resolve(true);
      });
    });

export async function fetchBrowserStatusAfterNow(
  dispatch: AppDispatch,
  getState: () => RootState,
  waitUntil: BrowserStatusWaiter,
): Promise<FreshBrowserStatus> {
  const deadline = Date.now() + BROWSER_STATUS_WAIT_MS;
  for (let attempt = 0; attempt < MAX_FRESH_STATUS_ATTEMPTS; attempt += 1) {
    if (isStatusPending(getState())) {
      const remaining = deadline - Date.now();
      if (
        remaining <= 0 ||
        !(await waitUntil((state) => !isStatusPending(state), remaining))
      )
        return { fetched: false };
    }
    const request = dispatch(
      browserApi.endpoints.projectBrowserStatus.initiate(undefined, {
        forceRefetch: true,
        subscribe: false,
      }),
    );
    const settled = await new Promise<boolean>((resolve) => {
      const timer = window.setTimeout(
        () => resolve(false),
        Math.max(0, deadline - Date.now()),
      );
      void request.then(() => {
        window.clearTimeout(timer);
        resolve(true);
      });
    });
    if (!settled) {
      request.abort();
      return { fetched: false };
    }
    const latest = selectStatusResult(getState());
    if (latest.requestId === request.requestId)
      return latest.isError
        ? { fetched: true, error: latest.error }
        : { fetched: true };
  }
  return { fetched: false };
}

export const selectUsableBrowserTabIds = createSelector(
  [selectProjectBrowserStatus, selectStableBrowserStatus],
  (status, stable): ReadonlySet<string> | null => {
    if (!status || (status.busy && !stable)) return null;
    return new Set(status.running ? status.tabs.map((tab) => tab.tab_id) : []);
  },
);

export const selectChatBrowserViewTabIds = createSelector(
  [selectBrowserViewOwners, (_state: RootState, chatId: string) => chatId],
  (owners, chatId) =>
    Object.entries(owners)
      .filter(([, owner]) => owner === chatId)
      .map(([tabId]) => tabId),
);

export const isBrowserViewUsable = (
  state: RootState,
  usable: ReadonlySet<string> | null,
  tabId: string,
): boolean =>
  !usable || usable.has(tabId) || selectBrowserTabAwaitingStatus(state, tabId);

export const selectChatHasUsableBrowserView = (
  state: RootState,
  chatId: string,
): boolean => {
  const usable = selectUsableBrowserTabIds(state);
  return selectChatBrowserViewTabIds(state, chatId).some((tabId) =>
    isBrowserViewUsable(state, usable, tabId),
  );
};

export const browserWorkspaceMiddleware: Middleware = (api) => {
  const dispatch = api.dispatch as AppDispatch;
  const getState = () => api.getState() as RootState;
  const pending = new Map<
    string,
    { chatId: string; tabId: string; automatic: boolean }
  >();
  const misses = new Map<string, number>();

  const show = (chatId: string, tabId: string, automatic: boolean) => {
    if (!isValidBrowserTabId(tabId)) {
      dispatch(setInformation(BROWSER_VIEW_INVALID_TAB_NOTICE));
      return;
    }
    const state = getState();
    if (!state.chat.open_thread_ids.includes(chatId)) return;
    if (!state.chat.threads[chatId]?.snapshot_received) {
      pending.set(`${chatId}\n${tabId}`, { chatId, tabId, automatic });
      return;
    }
    if (
      automatic &&
      isBrowserViewHeldByVisibleChat(state.workspace, chatId, tabId)
    )
      return;
    if (!canShowBrowserTabForChat(state.workspace, chatId, tabId)) {
      dispatch(setInformation(BROWSER_VIEW_CAP_NOTICE));
      return;
    }
    dispatch(showBrowserTabForChat({ chatId, tabId, automatic }));
  };

  const pruneStale = (status: ProjectBrowserStatus) => {
    if (!status.running || !status.connected || status.busy) return;
    const live = new Set(status.tabs.map((tab) => tab.tab_id));
    const state = getState();
    const owners = selectBrowserViewOwners(state);
    const showing = new Set(state.browserViews.showingChats);
    const keep: string[] = [];
    let stale = false;
    for (const [tabId, owner] of Object.entries(owners)) {
      if (live.has(tabId)) {
        misses.delete(tabId);
        keep.push(tabId);
        continue;
      }
      if (showing.has(owner)) {
        keep.push(tabId);
        continue;
      }
      const count = (misses.get(tabId) ?? 0) + 1;
      misses.set(tabId, count);
      if (count >= STALE_STATUS_MISSES) {
        misses.delete(tabId);
        stale = true;
      } else {
        keep.push(tabId);
      }
    }
    if (stale) dispatch(pruneBrowserTabSurfaces({ liveTabIds: keep }));
  };

  return (next) => (action) => {
    const result = next(action);
    if (openBrowserTabForChat.match(action)) {
      const state = getState();
      const chatId =
        action.payload.chatId ??
        selectFocusedWorkspaceChatId(state) ??
        (state.chat.open_thread_ids.includes(state.chat.current_thread_id)
          ? state.chat.current_thread_id
          : null);
      if (chatId)
        show(chatId, action.payload.tabId, action.payload.automatic === true);
      else dispatch(setInformation(BROWSER_VIEW_NO_CHAT_NOTICE));
    } else if (
      browserApi.endpoints.projectBrowserStatus.matchFulfilled(action)
    ) {
      pruneStale(action.payload);
    } else if (browserShowFinished.match(action)) {
      for (const [tabId, owner] of Object.entries(
        selectBrowserViewOwners(getState()),
      ))
        if (owner === action.payload) misses.delete(tabId);
    }
    for (const [key, intent] of pending) {
      const state = getState();
      if (!state.chat.open_thread_ids.includes(intent.chatId)) {
        pending.delete(key);
      } else if (state.chat.threads[intent.chatId]?.snapshot_received) {
        pending.delete(key);
        show(intent.chatId, intent.tabId, intent.automatic);
      }
    }
    return result;
  };
};
