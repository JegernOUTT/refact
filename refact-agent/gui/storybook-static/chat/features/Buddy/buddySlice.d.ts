import { weakMapMemoize, Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, ActionCreatorWithoutPayload, ActionCreatorWithOptionalPayload, PayloadAction } from '@reduxjs/toolkit';
import { ChatReactionDebug, BuddySnapshot, BuddyState, BuddyActivityEntry, BuddySuggestion, BuddySettings, BuddyConversationEntry, DiagnosticContext, BuddyRuntimeEvent, BuddySpeechItem, BuddyOpportunity, OpportunityStatus, BuddyPulse, BuddyDraft, BuddyStorageMetadata } from './types';
export type BuddyChatBubbleClass = "ambient" | "actionable" | "event_once";
export interface BuddyChatBubbleImpression {
    id: string;
    kind: BuddyChatBubbleClass;
    shown_at: number;
}
export declare function defaultBuddySettings(): BuddySettings;
export declare function normalizeBuddySettings(settings?: Partial<BuddySettings>): BuddySettings;
export type BuddySettingsPatch = Partial<BuddySettings> & {
    clear_personality_prompt?: boolean;
};
export type BuddySettingsPatchKey = keyof BuddySettings;
export type BuddySettingsResponse = BuddySettings & {
    storage?: BuddyStorageMetadata;
};
interface PendingBuddySettingsRequest {
    requestSeq: number;
    keys: BuddySettingsPatchKey[];
    patch: BuddySettingsPatch;
}
export declare function defaultBuddyPulse(): BuddyPulse;
export interface BuddySliceState {
    snapshot: BuddySnapshot | null;
    /** true once the first snapshot event has been received (even if buddy is disabled) */
    loaded: boolean;
    conversations: BuddyConversationEntry[];
    recentDiagnostics: DiagnosticContext[];
    runtimeQueue: BuddyRuntimeEvent[];
    nowPlaying: BuddyRuntimeEvent | null;
    activeSpeech: BuddySpeechItem | null;
    opportunities: BuddyOpportunity[];
    pulse: BuddyPulse | null;
    activeDrafts: BuddyDraft[];
    homeSnoozedUntil: number | null;
    seenNotificationIds: Record<string, number>;
    chatBubbleSnoozedUntil: number | null;
    chatBubbleImpressions: BuddyChatBubbleImpression[];
    pendingSettingsRequests: PendingBuddySettingsRequest[];
}
export declare const buddySlice: Slice<BuddySliceState, {
    setBuddySnapshot: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddySnapshot>) => void;
    /** Called when SSE snapshot reports buddy as disabled/not-ready (no state). */
    setBuddyUnavailable: (state: WritableDraft<BuddySliceState>) => void;
    updateBuddyState: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyState>) => void;
    addBuddyActivity: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyActivityEntry>) => void;
    addBuddySuggestion: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddySuggestion>) => void;
    dismissBuddySuggestion: (state: WritableDraft<BuddySliceState>, action: PayloadAction<string>) => void;
    updateBuddySettings: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddySettings>) => void;
    patchBuddySettings: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddySettingsPatch>) => void;
    beginBuddySettingsRequest: (state: WritableDraft<BuddySliceState>, action: PayloadAction<{
        requestSeq: number;
        keys: BuddySettingsPatchKey[];
        patch: BuddySettingsPatch;
    }>) => void;
    finishBuddySettingsRequest: (state: WritableDraft<BuddySliceState>, action: PayloadAction<{
        requestSeq: number;
        settings?: BuddySettingsResponse;
    }>) => void;
    failBuddySettingsRequest: (state: WritableDraft<BuddySliceState>, action: PayloadAction<{
        requestSeq: number;
        rollbackPatch: BuddySettingsPatch | null;
    }>) => void;
    setBuddyConversations: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyConversationEntry[]>) => void;
    addBuddyDiagnostic: (state: WritableDraft<BuddySliceState>, action: PayloadAction<DiagnosticContext>) => void;
    enqueueRuntimeEvent: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyRuntimeEvent>) => void;
    dequeueRuntimeEvent: (state: WritableDraft<BuddySliceState>) => void;
    clearNowPlaying: (state: WritableDraft<BuddySliceState>) => void;
    updateRuntimeProgress: (state: WritableDraft<BuddySliceState>, action: PayloadAction<{
        dedupe_key: string;
        progress: number;
    }>) => void;
    setActiveSpeech: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddySpeechItem>) => void;
    clearActiveSpeech: (state: WritableDraft<BuddySliceState>) => void;
    /** Mark a runtime event as dismissed by id (optimistic; server confirms via SSE). */
    dismissRuntimeEvent: (state: WritableDraft<BuddySliceState>, action: PayloadAction<string>) => void;
    addOpportunity: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyOpportunity>) => void;
    resolveOpportunity: (state: WritableDraft<BuddySliceState>, action: PayloadAction<{
        id: string;
        status: OpportunityStatus;
    }>) => void;
    expireOpportunities: (state: WritableDraft<BuddySliceState>, action: PayloadAction<string>) => void;
    setPulse: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyPulse>) => void;
    addDraft: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyDraft>) => void;
    consumeDraft: (state: WritableDraft<BuddySliceState>, action: PayloadAction<string>) => void;
    removeDraft: (state: WritableDraft<BuddySliceState>, action: PayloadAction<string>) => void;
    snoozeHomeNotifications: (state: WritableDraft<BuddySliceState>, action: PayloadAction<number | undefined>) => void;
    markBuddyNotificationSeen: (state: WritableDraft<BuddySliceState>, action: PayloadAction<string>) => void;
    recordChatBubbleImpression: (state: WritableDraft<BuddySliceState>, action: PayloadAction<{
        id: string;
        kind: BuddyChatBubbleClass;
    }>) => void;
    snoozeChatBubbles: (state: WritableDraft<BuddySliceState>, action: PayloadAction<number | undefined>) => void;
    clearExpiredChatBubbleSnooze: (state: WritableDraft<BuddySliceState>) => void;
    clearExpiredBuddyNotificationSnooze: (state: WritableDraft<BuddySliceState>) => void;
    replaceOpportunities: (state: WritableDraft<BuddySliceState>, action: PayloadAction<BuddyOpportunity[]>) => void;
}, "buddy", "buddy", {
    selectBuddySnapshot: (state: BuddySliceState) => BuddySnapshot | null;
    selectBuddyLoaded: (state: BuddySliceState) => boolean;
    selectBuddyState: (state: BuddySliceState) => BuddyState | null;
    selectBuddySettings: (state: BuddySliceState) => BuddySettings | null;
    selectBuddyStorage: (state: BuddySliceState) => BuddyStorageMetadata | null;
    selectBuddyActivities: (state: BuddySliceState) => BuddyActivityEntry[];
    selectBuddySuggestions: (state: BuddySliceState) => BuddySuggestion[];
    selectBuddyConversations: (state: BuddySliceState) => BuddyConversationEntry[];
    selectIsBuddySnapshotAvailable: (state: BuddySliceState) => boolean;
    selectIsBuddyUserEnabled: (state: BuddySliceState) => boolean;
    selectIsBuddyInteractiveEnabled: (state: BuddySliceState) => boolean;
    selectIsBuddyEnabled: (state: BuddySliceState) => boolean;
    selectBuddyDiagnostics: (state: BuddySliceState) => DiagnosticContext[];
    selectRuntimeQueue: (state: BuddySliceState) => BuddyRuntimeEvent[];
    selectNowPlaying: (state: BuddySliceState) => BuddyRuntimeEvent | null;
    selectActiveSpeech: (state: BuddySliceState) => BuddySpeechItem | null;
    selectOpportunities: (state: BuddySliceState) => BuddyOpportunity[];
    selectUnreadOpportunities: ((state: BuddySliceState) => BuddyOpportunity[]) & {
        clearCache: () => void;
        resultsCount: () => number;
        resetResultsCount: () => void;
    } & {
        resultFunc: (resultFuncArgs_0: BuddyOpportunity[]) => BuddyOpportunity[];
        memoizedResultFunc: ((resultFuncArgs_0: BuddyOpportunity[]) => BuddyOpportunity[]) & {
            clearCache: () => void;
            resultsCount: () => number;
            resetResultsCount: () => void;
        };
        lastResult: () => BuddyOpportunity[];
        dependencies: [(state: BuddySliceState) => BuddyOpportunity[]];
        recomputations: () => number;
        resetRecomputations: () => void;
        dependencyRecomputations: () => number;
        resetDependencyRecomputations: () => void;
    } & {
        memoize: typeof weakMapMemoize;
        argsMemoize: typeof weakMapMemoize;
    };
    selectPulse: (state: BuddySliceState) => BuddyPulse | null;
    selectActiveDrafts: (state: BuddySliceState) => BuddyDraft[];
    selectHomeSnoozedUntil: (state: BuddySliceState) => number | null;
    selectSeenNotificationIds: (state: BuddySliceState) => Record<string, number>;
    selectChatBubbleSnoozedUntil: (state: BuddySliceState) => number | null;
    selectChatBubbleImpressions: (state: BuddySliceState) => BuddyChatBubbleImpression[];
}>;
export declare const setBuddySnapshot: ActionCreatorWithPayload<BuddySnapshot, "buddy/setBuddySnapshot">, setBuddyUnavailable: ActionCreatorWithoutPayload<"buddy/setBuddyUnavailable">, updateBuddyState: ActionCreatorWithPayload<BuddyState, "buddy/updateBuddyState">, addBuddyActivity: ActionCreatorWithPayload<BuddyActivityEntry, "buddy/addBuddyActivity">, addBuddySuggestion: ActionCreatorWithPayload<BuddySuggestion, "buddy/addBuddySuggestion">, dismissBuddySuggestion: ActionCreatorWithPayload<string, "buddy/dismissBuddySuggestion">, updateBuddySettings: ActionCreatorWithPayload<BuddySettings, "buddy/updateBuddySettings">, patchBuddySettings: ActionCreatorWithPayload<BuddySettingsPatch, "buddy/patchBuddySettings">, beginBuddySettingsRequest: ActionCreatorWithPayload<{
    requestSeq: number;
    keys: BuddySettingsPatchKey[];
    patch: BuddySettingsPatch;
}, "buddy/beginBuddySettingsRequest">, finishBuddySettingsRequest: ActionCreatorWithPayload<{
    requestSeq: number;
    settings?: BuddySettingsResponse;
}, "buddy/finishBuddySettingsRequest">, failBuddySettingsRequest: ActionCreatorWithPayload<{
    requestSeq: number;
    rollbackPatch: BuddySettingsPatch | null;
}, "buddy/failBuddySettingsRequest">, setBuddyConversations: ActionCreatorWithPayload<BuddyConversationEntry[], "buddy/setBuddyConversations">, addBuddyDiagnostic: ActionCreatorWithPayload<DiagnosticContext, "buddy/addBuddyDiagnostic">, enqueueRuntimeEvent: ActionCreatorWithPayload<BuddyRuntimeEvent, "buddy/enqueueRuntimeEvent">, dequeueRuntimeEvent: ActionCreatorWithoutPayload<"buddy/dequeueRuntimeEvent">, clearNowPlaying: ActionCreatorWithoutPayload<"buddy/clearNowPlaying">, updateRuntimeProgress: ActionCreatorWithPayload<{
    dedupe_key: string;
    progress: number;
}, "buddy/updateRuntimeProgress">, setActiveSpeech: ActionCreatorWithPayload<BuddySpeechItem, "buddy/setActiveSpeech">, clearActiveSpeech: ActionCreatorWithoutPayload<"buddy/clearActiveSpeech">, dismissRuntimeEvent: ActionCreatorWithPayload<string, "buddy/dismissRuntimeEvent">, addOpportunity: ActionCreatorWithPayload<BuddyOpportunity, "buddy/addOpportunity">, resolveOpportunity: ActionCreatorWithPayload<{
    id: string;
    status: OpportunityStatus;
}, "buddy/resolveOpportunity">, expireOpportunities: ActionCreatorWithPayload<string, "buddy/expireOpportunities">, setPulse: ActionCreatorWithPayload<BuddyPulse, "buddy/setPulse">, addDraft: ActionCreatorWithPayload<BuddyDraft, "buddy/addDraft">, consumeDraft: ActionCreatorWithPayload<string, "buddy/consumeDraft">, removeDraft: ActionCreatorWithPayload<string, "buddy/removeDraft">, snoozeHomeNotifications: ActionCreatorWithOptionalPayload<number | undefined, "buddy/snoozeHomeNotifications">, markBuddyNotificationSeen: ActionCreatorWithPayload<string, "buddy/markBuddyNotificationSeen">, recordChatBubbleImpression: ActionCreatorWithPayload<{
    id: string;
    kind: BuddyChatBubbleClass;
}, "buddy/recordChatBubbleImpression">, snoozeChatBubbles: ActionCreatorWithOptionalPayload<number | undefined, "buddy/snoozeChatBubbles">, clearExpiredChatBubbleSnooze: ActionCreatorWithoutPayload<"buddy/clearExpiredChatBubbleSnooze">, clearExpiredBuddyNotificationSnooze: ActionCreatorWithoutPayload<"buddy/clearExpiredBuddyNotificationSnooze">, replaceOpportunities: ActionCreatorWithPayload<BuddyOpportunity[], "buddy/replaceOpportunities">;
export declare const selectBuddySnapshot: Selector<{
    buddy: BuddySliceState;
}, BuddySnapshot | null, []> & {
    unwrapped: (state: BuddySliceState) => BuddySnapshot | null;
}, selectBuddyLoaded: Selector<{
    buddy: BuddySliceState;
}, boolean, []> & {
    unwrapped: (state: BuddySliceState) => boolean;
}, selectBuddyState: Selector<{
    buddy: BuddySliceState;
}, BuddyState | null, []> & {
    unwrapped: (state: BuddySliceState) => BuddyState | null;
}, selectBuddySettings: Selector<{
    buddy: BuddySliceState;
}, BuddySettings | null, []> & {
    unwrapped: (state: BuddySliceState) => BuddySettings | null;
}, selectBuddyStorage: Selector<{
    buddy: BuddySliceState;
}, BuddyStorageMetadata | null, []> & {
    unwrapped: (state: BuddySliceState) => BuddyStorageMetadata | null;
}, selectBuddyActivities: Selector<{
    buddy: BuddySliceState;
}, BuddyActivityEntry[], []> & {
    unwrapped: (state: BuddySliceState) => BuddyActivityEntry[];
}, selectBuddySuggestions: Selector<{
    buddy: BuddySliceState;
}, BuddySuggestion[], []> & {
    unwrapped: (state: BuddySliceState) => BuddySuggestion[];
}, selectBuddyConversations: Selector<{
    buddy: BuddySliceState;
}, BuddyConversationEntry[], []> & {
    unwrapped: (state: BuddySliceState) => BuddyConversationEntry[];
}, selectIsBuddySnapshotAvailable: Selector<{
    buddy: BuddySliceState;
}, boolean, []> & {
    unwrapped: (state: BuddySliceState) => boolean;
}, selectIsBuddyUserEnabled: Selector<{
    buddy: BuddySliceState;
}, boolean, []> & {
    unwrapped: (state: BuddySliceState) => boolean;
}, selectIsBuddyInteractiveEnabled: Selector<{
    buddy: BuddySliceState;
}, boolean, []> & {
    unwrapped: (state: BuddySliceState) => boolean;
}, selectIsBuddyEnabled: Selector<{
    buddy: BuddySliceState;
}, boolean, []> & {
    unwrapped: (state: BuddySliceState) => boolean;
}, selectBuddyDiagnostics: Selector<{
    buddy: BuddySliceState;
}, DiagnosticContext[], []> & {
    unwrapped: (state: BuddySliceState) => DiagnosticContext[];
}, selectRuntimeQueue: Selector<{
    buddy: BuddySliceState;
}, BuddyRuntimeEvent[], []> & {
    unwrapped: (state: BuddySliceState) => BuddyRuntimeEvent[];
}, selectNowPlaying: Selector<{
    buddy: BuddySliceState;
}, BuddyRuntimeEvent | null, []> & {
    unwrapped: (state: BuddySliceState) => BuddyRuntimeEvent | null;
}, selectActiveSpeech: Selector<{
    buddy: BuddySliceState;
}, BuddySpeechItem | null, []> & {
    unwrapped: (state: BuddySliceState) => BuddySpeechItem | null;
}, selectOpportunities: Selector<{
    buddy: BuddySliceState;
}, BuddyOpportunity[], []> & {
    unwrapped: (state: BuddySliceState) => BuddyOpportunity[];
}, selectUnreadOpportunities: Selector<{
    buddy: BuddySliceState;
}, BuddyOpportunity[], []> & {
    unwrapped: ((state: BuddySliceState) => BuddyOpportunity[]) & {
        clearCache: () => void;
        resultsCount: () => number;
        resetResultsCount: () => void;
    } & {
        resultFunc: (resultFuncArgs_0: BuddyOpportunity[]) => BuddyOpportunity[];
        memoizedResultFunc: ((resultFuncArgs_0: BuddyOpportunity[]) => BuddyOpportunity[]) & {
            clearCache: () => void;
            resultsCount: () => number;
            resetResultsCount: () => void;
        };
        lastResult: () => BuddyOpportunity[];
        dependencies: [(state: BuddySliceState) => BuddyOpportunity[]];
        recomputations: () => number;
        resetRecomputations: () => void;
        dependencyRecomputations: () => number;
        resetDependencyRecomputations: () => void;
    } & {
        memoize: typeof weakMapMemoize;
        argsMemoize: typeof weakMapMemoize;
    };
}, selectPulse: Selector<{
    buddy: BuddySliceState;
}, BuddyPulse | null, []> & {
    unwrapped: (state: BuddySliceState) => BuddyPulse | null;
}, selectActiveDrafts: Selector<{
    buddy: BuddySliceState;
}, BuddyDraft[], []> & {
    unwrapped: (state: BuddySliceState) => BuddyDraft[];
}, selectHomeSnoozedUntil: Selector<{
    buddy: BuddySliceState;
}, number | null, []> & {
    unwrapped: (state: BuddySliceState) => number | null;
}, selectSeenNotificationIds: Selector<{
    buddy: BuddySliceState;
}, Record<string, number>, []> & {
    unwrapped: (state: BuddySliceState) => Record<string, number>;
}, selectChatBubbleSnoozedUntil: Selector<{
    buddy: BuddySliceState;
}, number | null, []> & {
    unwrapped: (state: BuddySliceState) => number | null;
}, selectChatBubbleImpressions: Selector<{
    buddy: BuddySliceState;
}, BuddyChatBubbleImpression[], []> & {
    unwrapped: (state: BuddySliceState) => BuddyChatBubbleImpression[];
};
export declare const selectOpportunityById: (state: {
    buddy: BuddySliceState;
}, id: string) => BuddyOpportunity | undefined;
export declare const selectDraftById: (state: {
    buddy: BuddySliceState;
}, id: string) => BuddyDraft | undefined;
export declare const selectChatReactionDebug: (state: {
    buddy: BuddySliceState;
}) => ChatReactionDebug | null;
export {};
