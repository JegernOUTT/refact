import { combineSlices, configureStore } from "@reduxjs/toolkit";
import { storage } from "./storage";
import { pruneStaleDraftMessages } from "../utils/threadStorage";
import {
  FLUSH,
  PAUSE,
  PERSIST,
  PURGE,
  REGISTER,
  REHYDRATE,
  persistReducer,
  persistStore,
} from "redux-persist";
import { statsApi } from "../services/refact/stats";
import { codeIntelApi } from "../services/refact/codeIntel";
import {
  capsApi,
  promptsApi,
  toolsApi,
  commandsApi,
  pathApi,
  pingApi,
  integrationsApi,
  knowledgeApi,
  knowledgeGraphApi,
  ragStatusApi,
  bugReportApi,
  providersApi,
  modelsApi,
  trajectoriesApi,
  trajectoryApi,
  tasksApi,
  taskMemoriesApi,
  taskDocumentsApi,
  browserApi,
  worktreesApi,
} from "../services/refact";
import { daemonApi } from "../services/refact/daemon";
import { chatModesApi } from "../services/refact/chatModes";
import { customizationApi } from "../services/refact/customization";
import { projectInformationApi } from "../services/refact/projectInformation";
import { setupStatusApi } from "../services/refact/setupStatus";
import { extensionsApi } from "../services/refact/extensions";
import { pluginsApi } from "../services/refact/plugins";
import { tipOfTheDaySlice } from "../features/TipOfTheDay";
import { reducer as configReducer } from "../features/Config/configSlice";
import { activeFileReducer } from "../features/Chat/activeFile";
import { selectedSnippetReducer } from "../features/Chat/selectedSnippet";
import { chatReducer } from "../features/Chat/Thread/reducer";
import {
  historySlice,
  historyMiddleware,
} from "../features/History/historySlice";
import { errorSlice } from "../features/Errors/errorsSlice";

import { pagesSlice } from "../features/Pages/pagesSlice";
import mergeInitialState from "redux-persist/lib/stateReconciler/autoMergeLevel2";
import { listenerMiddleware } from "./middleware";
import { informationSlice } from "../features/Errors/informationSlice";
import { linksApi } from "../services/refact/links";
import { integrationsSlice } from "../features/Integrations";
import { currentProjectInfoReducer } from "../features/Chat/currentProject";
import { checkpointsSlice } from "../features/Checkpoints/checkpointsSlice";
import { checkpointsApi } from "../services/refact/checkpoints";
import { patchesAndDiffsTrackerSlice } from "../features/PatchesAndDiffsTracker/patchesAndDiffsTrackerSlice";
import { tasksSlice } from "../features/Tasks";
import { knowledgeSlice } from "../features/Knowledge/knowledgeSlice";
import { connectionSlice } from "../features/Connection";
import { browserSlice } from "../features/Browser";
import { skillsStatusApi } from "../services/refact/skillsStatus";
import { mcpServerInfoApi } from "../services/refact/mcpServerInfo";
import { mcpMarketplaceApi } from "../services/refact/mcpMarketplace";
import { extensionsMarketplaceApi } from "../services/refact/extensionsMarketplace";
import { memoryEnrichmentApi } from "../services/refact/memoryEnrichment";
import { buddySlice } from "../features/Buddy/buddySlice";
import { sidebarReducer } from "../features/Sidebar/sidebarSlice";
import { buddyApi } from "../services/refact/buddy";
import { notificationsSlice } from "../features/Notifications";
import { schedulerSlice } from "../features/Scheduler";
import { schedulerApi } from "../services/refact/schedulerApi";
import { reconcileWorkspaceState, workspaceSlice } from "../features/Workspace";

const tipOfTheDayPersistConfig = {
  key: "totd",
  storage: storage(),
  stateReconciler: mergeInitialState,
};

const persistedTipOfTheDayReducer = persistReducer<
  ReturnType<typeof tipOfTheDaySlice.reducer>
>(tipOfTheDayPersistConfig, tipOfTheDaySlice.reducer);

// https://redux-toolkit.js.org/api/combineSlices
// `combineSlices` automatically combines the reducers using
// their `reducerPath`s, therefore we no longer need to call `combineReducers`.
const rootReducer = combineSlices(
  {
    // tipOfTheDay: persistedTipOfTheDayReducer,
    [tipOfTheDaySlice.reducerPath]: persistedTipOfTheDayReducer,
    config: configReducer,
    active_file: activeFileReducer,
    current_project: currentProjectInfoReducer,
    sidebar: sidebarReducer,
    selected_snippet: selectedSnippetReducer,
    chat: chatReducer,
    [statsApi.reducerPath]: statsApi.reducer,
    [codeIntelApi.reducerPath]: codeIntelApi.reducer,
    [capsApi.reducerPath]: capsApi.reducer,
    [promptsApi.reducerPath]: promptsApi.reducer,
    [toolsApi.reducerPath]: toolsApi.reducer,
    [commandsApi.reducerPath]: commandsApi.reducer,
    [pathApi.reducerPath]: pathApi.reducer,
    [pingApi.reducerPath]: pingApi.reducer,
    [linksApi.reducerPath]: linksApi.reducer,
    [checkpointsApi.reducerPath]: checkpointsApi.reducer,
    [knowledgeApi.reducerPath]: knowledgeApi.reducer,
    [knowledgeGraphApi.reducerPath]: knowledgeGraphApi.reducer,
    [ragStatusApi.reducerPath]: ragStatusApi.reducer,
    [bugReportApi.reducerPath]: bugReportApi.reducer,
    [providersApi.reducerPath]: providersApi.reducer,
    [modelsApi.reducerPath]: modelsApi.reducer,
    [trajectoriesApi.reducerPath]: trajectoriesApi.reducer,
    [trajectoryApi.reducerPath]: trajectoryApi.reducer,
    [tasksApi.reducerPath]: tasksApi.reducer,
    [schedulerApi.reducerPath]: schedulerApi.reducer,
    [taskMemoriesApi.reducerPath]: taskMemoriesApi.reducer,
    [taskDocumentsApi.reducerPath]: taskDocumentsApi.reducer,
    [browserApi.reducerPath]: browserApi.reducer,
    [worktreesApi.reducerPath]: worktreesApi.reducer,
    [daemonApi.reducerPath]: daemonApi.reducer,
    [skillsStatusApi.reducerPath]: skillsStatusApi.reducer,
    [mcpServerInfoApi.reducerPath]: mcpServerInfoApi.reducer,
    [chatModesApi.reducerPath]: chatModesApi.reducer,
    [customizationApi.reducerPath]: customizationApi.reducer,
    [projectInformationApi.reducerPath]: projectInformationApi.reducer,
    [setupStatusApi.reducerPath]: setupStatusApi.reducer,
    [extensionsApi.reducerPath]: extensionsApi.reducer,
    [pluginsApi.reducerPath]: pluginsApi.reducer,
    [mcpMarketplaceApi.reducerPath]: mcpMarketplaceApi.reducer,
    [extensionsMarketplaceApi.reducerPath]: extensionsMarketplaceApi.reducer,
    [memoryEnrichmentApi.reducerPath]: memoryEnrichmentApi.reducer,
    [buddyApi.reducerPath]: buddyApi.reducer,
  },
  historySlice,
  buddySlice,
  errorSlice,
  informationSlice,
  pagesSlice,
  integrationsApi,
  integrationsSlice,
  checkpointsSlice,
  patchesAndDiffsTrackerSlice,
  tasksSlice,
  knowledgeSlice,
  connectionSlice,
  browserSlice,
  notificationsSlice,
  schedulerSlice,
  workspaceSlice,
);

const rootPersistConfig = {
  key: "root",
  storage: storage(),
  whitelist: [],
  stateReconciler: mergeInitialState,
};

const APPLY_CHAT_EVENT_ACTION = "chatThread/applyChatEvent";

const workspaceInvariantReducer = (state: ReturnType<typeof rootReducer>) => {
  const nextWorkspace = reconcileWorkspaceState(
    state.workspace,
    state.chat.open_thread_ids,
  );

  if (nextWorkspace === state.workspace) {
    return state;
  }

  return {
    ...state,
    workspace: nextWorkspace,
  };
};

const persistedReducer = persistReducer<ReturnType<typeof rootReducer>>(
  rootPersistConfig,
  (state, action) => workspaceInvariantReducer(rootReducer(state, action)),
);

export type RootState = ReturnType<typeof persistedReducer>;

export function setUpStore(preloadedState?: Partial<RootState>) {
  const initialState = {
    ...preloadedState,
    ...window.__INITIAL_STATE__,
  } as RootState;

  const store = configureStore({
    reducer: persistedReducer,
    preloadedState: initialState,
    devTools: {
      maxAge: 50,
    },
    middleware: (getDefaultMiddleware) => {
      const production = import.meta.env.MODE === "production";
      const middleware = production
        ? getDefaultMiddleware({
            thunk: true,
            serializableCheck: false,
            immutableCheck: false,
          })
        : getDefaultMiddleware({
            serializableCheck: {
              ignoredActions: [
                FLUSH,
                REHYDRATE,
                PAUSE,
                PERSIST,
                PURGE,
                REGISTER,
                APPLY_CHAT_EVENT_ACTION,
              ],
              ignoredPaths: ["chat.threads"],
            },
            immutableCheck: {
              ignoredPaths: ["chat.threads"],
            },
          });

      return middleware
        .prepend(
          pingApi.middleware,
          statsApi.middleware,
          codeIntelApi.middleware,
          capsApi.middleware,
          promptsApi.middleware,
          toolsApi.middleware,
          commandsApi.middleware,
          pathApi.middleware,
          linksApi.middleware,
          integrationsApi.middleware,
          checkpointsApi.middleware,
          knowledgeApi.middleware,
          knowledgeGraphApi.middleware,
          ragStatusApi.middleware,
          bugReportApi.middleware,
          providersApi.middleware,
          modelsApi.middleware,
          trajectoriesApi.middleware,
          trajectoryApi.middleware,
          tasksApi.middleware,
          schedulerApi.middleware,
          taskMemoriesApi.middleware,
          taskDocumentsApi.middleware,
          browserApi.middleware,
          worktreesApi.middleware,
          daemonApi.middleware,
          skillsStatusApi.middleware,
          chatModesApi.middleware,
          customizationApi.middleware,
          projectInformationApi.middleware,
          setupStatusApi.middleware,
          extensionsApi.middleware,
          pluginsApi.middleware,
          mcpServerInfoApi.middleware,
          mcpMarketplaceApi.middleware,
          extensionsMarketplaceApi.middleware,
          memoryEnrichmentApi.middleware,
          buddyApi.middleware,
        )
        .prepend(historyMiddleware.middleware)
        .prepend(listenerMiddleware.middleware);
    },
  });

  return store;
}

pruneStaleDraftMessages();

export const store = setUpStore();
export type Store = typeof store;

export const persistor = persistStore(store);
// TODO: sync storage across windows (was buggy when deleting).
// window.onstorage = (event) => {
//   if (!event.key || !event.key.endsWith(persistConfig.key)) {
//     return;
//   }

//   if (event.oldValue === event.newValue) {
//     return;
//   }
//   if (event.newValue === null) {
//     return;
//   }

// Infer the `RootState` and `AppDispatch` types from the store itself
// export type RootState = ReturnType<typeof store.getState>;
// Inferred type: {posts: PostsState, comments: CommentsState, users: UsersState}
export type AppDispatch = typeof store.dispatch;

// Infer the type of `store`
export type AppStore = typeof store;

declare global {
  interface Window {
    __INITIAL_STATE__?: RootState;
  }
}
