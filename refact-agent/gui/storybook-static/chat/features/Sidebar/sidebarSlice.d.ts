import { Reducer } from 'redux';
import { ActionCreatorWithPayload } from '@reduxjs/toolkit';
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
export declare const sidebarSubscriptionStarted: ActionCreatorWithPayload<{
    subscriptionId: string | null;
    lspPort: number;
}, string>;
export declare const sidebarSectionSnapshotReceived: ActionCreatorWithPayload<{
    section: SidebarSection;
    status: Exclude<SidebarSectionStatus, "loading">;
    error?: string | null;
}, string>;
export declare const resetSidebarState: ActionCreatorWithPayload<{
    lspPort?: number | null;
}, string>;
export declare const sidebarWorkspaceChanged: ActionCreatorWithPayload<{
    subscriptionId: string | null;
}, string>;
export declare const sidebarReducer: Reducer<SidebarState> & {
    getInitialState: () => SidebarState;
};
export declare const selectSidebarSection: (section: SidebarSection) => (state: RootState) => SidebarSectionState;
export declare const selectWorkspaceSection: (state: RootState) => SidebarSectionState;
export declare const selectChatsSection: (state: RootState) => SidebarSectionState;
export declare const selectTasksSection: (state: RootState) => SidebarSectionState;
export declare const selectBuddySection: (state: RootState) => SidebarSectionState;
export declare const selectSidebarSubscriptionId: (state: RootState) => string | null;
export {};
