import { createAction, createReducer } from "@reduxjs/toolkit";
import type { RootState } from "../../app/store";

export type SidebarSection = "workspace" | "chats" | "tasks" | "buddy";
export type SidebarSectionStatus = "loading" | "ready" | "error";

type SidebarSectionState = {
  status: SidebarSectionStatus;
  error: string | null;
};

export type SidebarState = {
  subscriptionId: string | null;
  lspPort: number | null;
  sections: Record<SidebarSection, SidebarSectionState>;
};

const loadingSection = (): SidebarSectionState => ({
  status: "loading",
  error: null,
});

const initialState: SidebarState = {
  subscriptionId: null,
  lspPort: null,
  sections: {
    workspace: loadingSection(),
    chats: loadingSection(),
    tasks: loadingSection(),
    buddy: loadingSection(),
  },
};

export const sidebarSubscriptionStarted = createAction<{
  subscriptionId: string | null;
  lspPort: number;
}>("sidebar/subscriptionStarted");

export const sidebarSectionSnapshotReceived = createAction<{
  section: SidebarSection;
  status: Exclude<SidebarSectionStatus, "loading">;
  error?: string | null;
}>("sidebar/sectionSnapshotReceived");

export const resetSidebarState = createAction<{ lspPort?: number | null }>(
  "sidebar/reset",
);

export const sidebarWorkspaceChanged = createAction<{
  subscriptionId: string | null;
}>("sidebar/workspaceChanged");

export const sidebarReducer = createReducer(initialState, (builder) => {
  builder
    .addCase(sidebarSubscriptionStarted, (state, action) => {
      state.subscriptionId = action.payload.subscriptionId;
      state.lspPort = action.payload.lspPort;
      state.sections.workspace = loadingSection();
      state.sections.chats = loadingSection();
      state.sections.tasks = loadingSection();
      state.sections.buddy = loadingSection();
    })
    .addCase(sidebarSectionSnapshotReceived, (state, action) => {
      state.sections[action.payload.section] = {
        status: action.payload.status,
        error: action.payload.error ?? null,
      };
    })
    .addCase(sidebarWorkspaceChanged, (state, action) => {
      state.subscriptionId = action.payload.subscriptionId;
      state.sections.workspace = loadingSection();
      state.sections.chats = loadingSection();
      state.sections.tasks = loadingSection();
      state.sections.buddy = loadingSection();
    })
    .addCase(resetSidebarState, (state, action) => {
      state.subscriptionId = null;
      state.lspPort = action.payload.lspPort ?? null;
      state.sections.workspace = loadingSection();
      state.sections.chats = loadingSection();
      state.sections.tasks = loadingSection();
      state.sections.buddy = loadingSection();
    });
});

export const selectSidebarSection =
  (section: SidebarSection) =>
  (state: RootState): SidebarSectionState =>
    state.sidebar.sections[section];

export const selectWorkspaceSection = selectSidebarSection("workspace");
export const selectChatsSection = selectSidebarSection("chats");
export const selectTasksSection = selectSidebarSection("tasks");
export const selectBuddySection = selectSidebarSection("buddy");

export const selectSidebarSubscriptionId = (state: RootState): string | null =>
  state.sidebar.subscriptionId;
