import {
  createAction,
  createSlice,
  type PayloadAction,
} from "@reduxjs/toolkit";

export const openRailApp = createAction<{ appId: string; name: string }>(
  "apps/openRailApp",
);
export const retryAppWorkspaceSave = createAction<string>(
  "apps/retryWorkspaceSave",
);

export type AppViewBinding = {
  appId: string;
  bindingId?: string;
  chatId?: string;
  scopeChatId?: string;
  name: string;
};
export type StagedAppContext = {
  id: string;
  app_id: string;
  binding_id: string;
  revision: string;
  title: string;
  text: string;
  data?: unknown;
  view_id?: string;
  captured_at?: string;
  source_refs?: { path: string }[];
};
export function canStageAppContext(
  contexts: StagedAppContext[],
  context: StagedAppContext,
): boolean {
  const next = [...contexts.filter((item) => item.id !== context.id), context];
  try {
    return (
      context.text.length <= 8192 &&
      context.title.length <= 200 &&
      next.length <= 8 &&
      JSON.stringify(next).length <= 65536
    );
  } catch {
    return false;
  }
}
type AppsState = {
  pendingBindings?: Record<string, boolean | undefined>;
  saveStatus?: Record<
    string,
    "saving" | "saved" | "error" | "readonly" | undefined
  >;
  views: Record<string, AppViewBinding | undefined>;
  stagedByChat: Record<string, StagedAppContext[] | undefined>;
};
const initialState: AppsState = { views: {}, stagedByChat: {} };
export const appsSlice = createSlice({
  name: "apps",
  initialState,
  reducers: {
    appBindingPending(
      state,
      action: PayloadAction<{ bindingId: string; pending: boolean }>,
    ) {
      state.pendingBindings ??= {};
      state.pendingBindings[action.payload.bindingId] = action.payload.pending;
    },
    workspaceSaveStatus(
      state,
      action: PayloadAction<{
        chatId: string;
        status: "saving" | "saved" | "error" | "readonly";
      }>,
    ) {
      state.saveStatus ??= {};
      state.saveStatus[action.payload.chatId] = action.payload.status;
    },
    appViewOpened(
      state,
      action: PayloadAction<{ key: string; view: AppViewBinding }>,
    ) {
      state.views[action.payload.key] = action.payload.view;
    },
    appContextStaged(
      state,
      action: PayloadAction<{ chatId: string; context: StagedAppContext }>,
    ) {
      const { chatId, context } = action.payload;
      const contexts = state.stagedByChat[chatId] ?? [];
      if (!canStageAppContext(contexts, context)) return;
      state.stagedByChat[chatId] = [
        ...contexts.filter((item) => item.id !== context.id),
        context,
      ];
    },
    appContextRemoved(
      state,
      action: PayloadAction<{ chatId: string; id: string }>,
    ) {
      state.stagedByChat[action.payload.chatId] = state.stagedByChat[
        action.payload.chatId
      ]?.filter((item) => item.id !== action.payload.id);
    },
    appContextsConsumed(
      state,
      action: PayloadAction<{ chatId: string; ids: string[] }>,
    ) {
      state.stagedByChat[action.payload.chatId] = state.stagedByChat[
        action.payload.chatId
      ]?.filter((item) => !action.payload.ids.includes(item.id));
    },
    appContextsCleared(state, action: PayloadAction<string>) {
      state.stagedByChat[action.payload] = undefined;
    },
  },
});
export const {
  appViewOpened,
  appBindingPending,
  appContextStaged,
  appContextRemoved,
  appContextsCleared,
  appContextsConsumed,
} = appsSlice.actions;
