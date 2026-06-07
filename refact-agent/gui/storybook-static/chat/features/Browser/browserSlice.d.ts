import { WritableDraft } from 'immer';
import { Slice, SliceSelectors, ActionCreatorWithPayload, PayloadAction } from '@reduxjs/toolkit';
import { RootState } from "../../app/store";
export type DiffBox = {
    x: number;
    y: number;
    width: number;
    height: number;
};
export type BrowserTabInfo = {
    tab_id: string;
    url: string;
    title: string;
};
export type BrowserFrame = {
    mime: string;
    data: string;
    diff_boxes: DiffBox[];
};
export type TimelineEntry = {
    timestamp: string;
    source: "user" | "agent";
    type: string;
    summary: string;
    details?: Record<string, unknown>;
};
export type TimelineFilterSource = "all" | "user" | "agent";
export type BrowserNotification = {
    type: "detached" | "attached" | "closed" | "timeout";
    message: string;
};
export type BrowserContextOversizeInfo = {
    pending_message_id: string;
    total_bytes: number;
    action_count: number;
    action_bytes: number;
    console_count: number;
    console_bytes: number;
    network_count: number;
    network_bytes: number;
    mutation_bytes: number;
};
export type BrowserToolbarActionType = "screenshot" | "screenshot_full" | "pick_element" | "paste_actions" | "paste_console" | "paste_network" | "curl" | "summarize" | "extract_json" | "annotate" | "annotate_send" | "annotate_clear" | "rect_highlight";
export type BrowserRuntime = {
    runtime_id: string;
    connected: boolean;
    active_tab: string | null;
    url: string | null;
    title: string | null;
    tabs: BrowserTabInfo[];
    latest_frame: BrowserFrame | null;
    picker_active: boolean;
    annotate_active: boolean;
    attach_screenshot_on_send: boolean;
    timeline: TimelineEntry[];
    timeline_open: boolean;
    timeline_filter_source: TimelineFilterSource;
    timeline_filter_type: string | null;
    notification: BrowserNotification | null;
    oversize_info: BrowserContextOversizeInfo | null;
    pending_toolbar_actions: BrowserToolbarActionType[];
};
export type BrowserState = {
    runtimes: Record<string, BrowserRuntime | undefined>;
    browserUiOpen: Record<string, boolean>;
};
export declare function makeBrowserRuntime(runtime_id: string): BrowserRuntime;
export declare const browserSlice: Slice<BrowserState, {
    setBrowserRuntime(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        runtime: BrowserRuntime;
    }>): void;
    updateBrowserStatus(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        connected: boolean;
        url?: string | null;
        title?: string | null;
    }>): void;
    updateBrowserFrame(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        frame: BrowserFrame;
    }>): void;
    removeBrowserRuntime(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    setPickerActive(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        active: boolean;
    }>): void;
    setAnnotateActive(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        active: boolean;
    }>): void;
    toggleAttachScreenshotOnSend(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    addTimelineEntries(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        entries: TimelineEntry[];
    }>): void;
    clearTimeline(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    toggleTimelineOpen(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    setTimelineFilterSource(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        source: TimelineFilterSource;
    }>): void;
    setTimelineFilterType(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        type: string | null;
    }>): void;
    setBrowserNotification(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        notification: BrowserNotification | null;
    }>): void;
    markBrowserDetached(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    markBrowserClosed(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        reason: string;
    }>): void;
    setBrowserContextOversize(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
        info: BrowserContextOversizeInfo;
    }>): void;
    clearBrowserContextOversize(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    shiftPendingToolbarAction(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    openBrowserUi(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
    closeBrowserUi(state: WritableDraft<BrowserState>, action: PayloadAction<{
        chatId: string;
    }>): void;
}, "browser", "browser", SliceSelectors<BrowserState>>;
export declare const setBrowserRuntime: ActionCreatorWithPayload<{
    chatId: string;
    runtime: BrowserRuntime;
}, "browser/setBrowserRuntime">, updateBrowserStatus: ActionCreatorWithPayload<{
    chatId: string;
    connected: boolean;
    url?: string | null;
    title?: string | null;
}, "browser/updateBrowserStatus">, updateBrowserFrame: ActionCreatorWithPayload<{
    chatId: string;
    frame: BrowserFrame;
}, "browser/updateBrowserFrame">, removeBrowserRuntime: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/removeBrowserRuntime">, setPickerActive: ActionCreatorWithPayload<{
    chatId: string;
    active: boolean;
}, "browser/setPickerActive">, setAnnotateActive: ActionCreatorWithPayload<{
    chatId: string;
    active: boolean;
}, "browser/setAnnotateActive">, toggleAttachScreenshotOnSend: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/toggleAttachScreenshotOnSend">, addTimelineEntries: ActionCreatorWithPayload<{
    chatId: string;
    entries: TimelineEntry[];
}, "browser/addTimelineEntries">, clearTimeline: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/clearTimeline">, toggleTimelineOpen: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/toggleTimelineOpen">, setTimelineFilterSource: ActionCreatorWithPayload<{
    chatId: string;
    source: TimelineFilterSource;
}, "browser/setTimelineFilterSource">, setTimelineFilterType: ActionCreatorWithPayload<{
    chatId: string;
    type: string | null;
}, "browser/setTimelineFilterType">, setBrowserNotification: ActionCreatorWithPayload<{
    chatId: string;
    notification: BrowserNotification | null;
}, "browser/setBrowserNotification">, markBrowserDetached: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/markBrowserDetached">, markBrowserClosed: ActionCreatorWithPayload<{
    chatId: string;
    reason: string;
}, "browser/markBrowserClosed">, setBrowserContextOversize: ActionCreatorWithPayload<{
    chatId: string;
    info: BrowserContextOversizeInfo;
}, "browser/setBrowserContextOversize">, clearBrowserContextOversize: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/clearBrowserContextOversize">, shiftPendingToolbarAction: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/shiftPendingToolbarAction">, openBrowserUi: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/openBrowserUi">, closeBrowserUi: ActionCreatorWithPayload<{
    chatId: string;
}, "browser/closeBrowserUi">;
export declare const selectBrowserRuntime: (state: RootState, chatId: string) => BrowserRuntime | undefined;
export declare const selectBrowserRuntimes: (state: RootState) => Record<string, BrowserRuntime | undefined>;
export declare const selectTimeline: (state: RootState, chatId: string) => TimelineEntry[];
export declare const selectTimelineOpen: (state: RootState, chatId: string) => boolean;
export declare const selectTimelineFilterSource: (state: RootState, chatId: string) => TimelineFilterSource;
export declare const selectTimelineFilterType: (state: RootState, chatId: string) => string | null;
export declare const selectBrowserContextOversize: (state: RootState, chatId: string) => BrowserContextOversizeInfo | null;
export declare const selectBrowserUiOpen: (state: RootState, chatId: string) => boolean;
