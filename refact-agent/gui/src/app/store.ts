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
import { statisticsApi } from "../services/refact/statistics";
import { statsApi } from "../services/refact/stats";
import {
  capsApi,
  promptsApi,
  toolsApi,
  commandsApi,
  pathApi,
  pingApi,
  integrationsApi,
  telemetryApi,
  knowledgeApi,
  knowledgeGraphApi,
  providersApi,
  modelsApi,
  teamsApi,
  trajectoriesApi,
  trajectoryApi,
  tasksApi,
  browserApi,
} from "../services/refact";
import { chatModesApi } from "../services/refact/chatModes";
import { customizationApi } from "../services/refact/customization";
import { projectInformationApi } from "../services/refact/projectInformation";
import { extensionsApi } from "../services/refact/extensions";
import { pluginsApi } from "../services/refact/plugins";
import { smallCloudApi } from "../services/smallcloud";
import { reducer as fimReducer } from "../features/FIM/reducer";
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
import { teamsSlice } from "../features/Teams";
import { userSurveySlice } from "../features/UserSurvey/userSurveySlice";
import { linksApi } from "../services/refact/links";
import { integrationsSlice } from "../features/Integrations";
import { currentProjectInfoReducer } from "../features/Chat/currentProject";
import { checkpointsSlice } from "../features/Checkpoints/checkpointsSlice";
import { checkpointsApi } from "../services/refact/checkpoints";
import { patchesAndDiffsTrackerSlice } from "../features/PatchesAndDiffsTracker/patchesAndDiffsTrackerSlice";
import { coinBallanceSlice } from "../features/CoinBalance";
import { tasksSlice } from "../features/Tasks";
import { connectionSlice } from "../features/Connection";
import { browserSlice } from "../features/Browser";
import { skillsStatusApi } from "../services/refact/skillsStatus";
import { mcpServerInfoApi } from "../services/refact/mcpServerInfo";
import { mcpMarketplaceApi } from "../services/refact/mcpMarketplace";

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
    fim: fimReducer,
    // tipOfTheDay: persistedTipOfTheDayReducer,
    [tipOfTheDaySlice.reducerPath]: persistedTipOfTheDayReducer,
    config: configReducer,
    active_file: activeFileReducer,
    current_project: currentProjectInfoReducer,
    selected_snippet: selectedSnippetReducer,
    chat: chatReducer,
    [statisticsApi.reducerPath]: statisticsApi.reducer,
    [statsApi.reducerPath]: statsApi.reducer,
    [capsApi.reducerPath]: capsApi.reducer,
    [promptsApi.reducerPath]: promptsApi.reducer,
    [toolsApi.reducerPath]: toolsApi.reducer,
    [commandsApi.reducerPath]: commandsApi.reducer,
    [smallCloudApi.reducerPath]: smallCloudApi.reducer,
    [pathApi.reducerPath]: pathApi.reducer,
    [pingApi.reducerPath]: pingApi.reducer,
    [linksApi.reducerPath]: linksApi.reducer,
    [checkpointsApi.reducerPath]: checkpointsApi.reducer,
    [telemetryApi.reducerPath]: telemetryApi.reducer,
    [knowledgeApi.reducerPath]: knowledgeApi.reducer,
    [knowledgeGraphApi.reducerPath]: knowledgeGraphApi.reducer,
    [teamsApi.reducerPath]: teamsApi.reducer,
    [providersApi.reducerPath]: providersApi.reducer,
    [modelsApi.reducerPath]: modelsApi.reducer,
    [trajectoriesApi.reducerPath]: trajectoriesApi.reducer,
    [trajectoryApi.reducerPath]: trajectoryApi.reducer,
    [tasksApi.reducerPath]: tasksApi.reducer,
    [browserApi.reducerPath]: browserApi.reducer,
    [skillsStatusApi.reducerPath]: skillsStatusApi.reducer,
    [mcpServerInfoApi.reducerPath]: mcpServerInfoApi.reducer,
    [chatModesApi.reducerPath]: chatModesApi.reducer,
    [customizationApi.reducerPath]: customizationApi.reducer,
    [projectInformationApi.reducerPath]: projectInformationApi.reducer,
    [extensionsApi.reducerPath]: extensionsApi.reducer,
    [pluginsApi.reducerPath]: pluginsApi.reducer,
    [mcpMarketplaceApi.reducerPath]: mcpMarketplaceApi.reducer,

  },
  historySlice,
  errorSlice,
  informationSlice,
  pagesSlice,
  integrationsApi,
  userSurveySlice,
  teamsSlice,
  integrationsSlice,
  checkpointsSlice,
  patchesAndDiffsTrackerSlice,
  coinBallanceSlice,
  tasksSlice,
  connectionSlice,
  browserSlice,
);

const rootPersistConfig = {
  key: "root",
  storage: storage(),
  whitelist: [userSurveySlice.reducerPath],
  stateReconciler: mergeInitialState,
};

const persistedReducer = persistReducer<ReturnType<typeof rootReducer>>(
  rootPersistConfig,
  rootReducer,
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
              ],
            },
          });

      return middleware
        .prepend(
          pingApi.middleware,
          statisticsApi.middleware,
          statsApi.middleware,
          capsApi.middleware,
          promptsApi.middleware,
          toolsApi.middleware,
          commandsApi.middleware,
          smallCloudApi.middleware,
          pathApi.middleware,
          linksApi.middleware,
          integrationsApi.middleware,
          checkpointsApi.middleware,
          telemetryApi.middleware,
          knowledgeApi.middleware,
          knowledgeGraphApi.middleware,
          providersApi.middleware,
          modelsApi.middleware,
          teamsApi.middleware,
          trajectoriesApi.middleware,
          trajectoryApi.middleware,
          tasksApi.middleware,
          browserApi.middleware,
          skillsStatusApi.middleware,
          chatModesApi.middleware,
          customizationApi.middleware,
          projectInformationApi.middleware,
          extensionsApi.middleware,
          pluginsApi.middleware,
          mcpServerInfoApi.middleware,
          mcpMarketplaceApi.middleware,

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
