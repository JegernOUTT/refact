import { createReducer, createAction, createSelector } from "@reduxjs/toolkit";
import { type ThemeProps } from "../../components/Theme";
import { RootState } from "../../app/store";

export type RefactBackendConnectionStatus =
  | "connecting"
  | "starting"
  | "installing"
  | "ready"
  | "failed";

export type Config = {
  host: "web" | "ide" | "vscode" | "jetbrains";
  lspPort: number;
  tabbed?: boolean;
  lspUrl?: string;
  browserUrl?: string;
  dev?: boolean;
  engineServed?: boolean;
  backendReady?: boolean;
  connectionStatus?: RefactBackendConnectionStatus;
  // todo: handle light / darkmode
  themeProps: Omit<ThemeProps, "children">;
  features?: {
    statistics?: boolean;
    vecdb?: boolean;
    ast?: boolean;
    codegraph?: boolean;
    images?: boolean;
  };
  keyBindings?: {
    completeManual?: string;
  };
  apiKey?: string | null;
  shiftEnterToSubmit?: boolean;
  currentWorkspaceName?: string;
};

const initialState: Config = {
  host: "web",
  lspPort: __REFACT_LSP_PORT__ ?? 8001,
  apiKey: null,
  features: {
    statistics: true,
    vecdb: true,
    ast: true,
    codegraph: true,
    images: true,
  },
  themeProps: {
    appearance: "dark",
  },
  shiftEnterToSubmit: false,
};

export type ConfigUpdate = Omit<Partial<Config>, "lspUrl" | "browserUrl"> & {
  lspUrl?: string | null;
  browserUrl?: string | null;
};

export const updateConfig = createAction<ConfigUpdate>("config/update");

export const setThemeMode = createAction<"light" | "dark" | "inherit">(
  "config/setThemeMode",
);
export const setApiKey = createAction<string | null>("config/setApiKey");

export const changeFeature = createAction<{
  feature: string;
  value: boolean;
}>("config/feature/change");

function hasConfigProperty(
  config: Partial<Record<keyof Config, unknown>>,
  key: keyof Config,
): boolean {
  return Object.prototype.hasOwnProperty.call(config, key);
}

export const reducer = createReducer<Config>(initialState, (builder) => {
  // TODO: toggle darkmode for web host?
  builder.addCase(updateConfig, (state, action) => {
    state.dev = action.payload.dev ?? state.dev;
    state.engineServed = action.payload.engineServed ?? state.engineServed;
    if (hasConfigProperty(action.payload, "backendReady")) {
      if (action.payload.backendReady === undefined) {
        delete state.backendReady;
      } else {
        state.backendReady = action.payload.backendReady;
      }
    }
    if (hasConfigProperty(action.payload, "connectionStatus")) {
      if (action.payload.connectionStatus === undefined) {
        delete state.connectionStatus;
      } else {
        state.connectionStatus = action.payload.connectionStatus;
      }
    }

    state.features = action.payload.features
      ? { ...state.features, ...action.payload.features }
      : state.features;

    state.host = action.payload.host ?? state.host;
    if (hasConfigProperty(action.payload, "lspUrl")) {
      if (
        action.payload.lspUrl === undefined ||
        action.payload.lspUrl === null
      ) {
        delete state.lspUrl;
      } else {
        state.lspUrl = action.payload.lspUrl;
      }
    }
    if (hasConfigProperty(action.payload, "browserUrl")) {
      if (
        action.payload.browserUrl === undefined ||
        action.payload.browserUrl === null
      ) {
        delete state.browserUrl;
      } else {
        state.browserUrl = action.payload.browserUrl;
      }
    }
    state.tabbed = action.payload.tabbed ?? state.tabbed;
    state.themeProps = action.payload.themeProps
      ? { ...state.themeProps, ...action.payload.themeProps }
      : state.themeProps;
    state.apiKey = action.payload.apiKey ?? state.apiKey;
    state.lspPort = action.payload.lspPort ?? state.lspPort;
    state.keyBindings = action.payload.keyBindings ?? state.keyBindings;
    state.currentWorkspaceName =
      action.payload.currentWorkspaceName ?? state.currentWorkspaceName;
    state.shiftEnterToSubmit =
      action.payload.shiftEnterToSubmit ?? state.shiftEnterToSubmit;
  });

  builder.addCase(setThemeMode, (state, action) => {
    state.themeProps.appearance = action.payload;
  });

  builder.addCase(setApiKey, (state, action) => {
    state.apiKey = action.payload;
  });

  builder.addCase(changeFeature, (state, action) => {
    state.features = {
      ...(state.features ?? {}),
      [action.payload.feature]: action.payload.value,
    };
  });
});

export const selectThemeMode = (state: RootState) =>
  state.config.themeProps.appearance;

export const selectConfig = (state: RootState) => state.config;
export const selectLspPort = (state: RootState) => state.config.lspPort;

export const selectFeatures = (state: RootState) => state.config.features;
export const selectVecdb = createSelector(
  selectFeatures,
  (features) => features?.vecdb,
);
export const selectAst = createSelector(
  selectFeatures,
  (features) => features?.ast,
);
export const selectCodegraph = createSelector(
  selectFeatures,
  (features) => features?.codegraph,
);

export const selectApiKey = (state: RootState) => state.config.apiKey;
export const selectHost = (state: RootState) => state.config.host;
export const selectSubmitOption = (state: RootState) =>
  state.config.shiftEnterToSubmit ?? false;
