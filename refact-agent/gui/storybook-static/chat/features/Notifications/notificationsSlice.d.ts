import { weakMapMemoize } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, SliceSelectors, ActionCreatorWithPreparedPayload, ActionCreatorWithoutPayload, type PayloadAction } from '@reduxjs/toolkit';
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";
export type ProcessCompletedEvent = Extract<ChatEventEnvelope, {
    type: "process_completed";
}>;
export type ProcessCompletedNotification = {
    id: string;
    threadId: string;
    seq: string;
    processId: string;
    status: string;
    exitCode: number | null;
    shortDescription: string;
    mode: string;
    receivedAt: number;
};
export type NotificationsState = {
    pendingByThread: Partial<Record<string, ProcessCompletedNotification[]>>;
    lastSeenByThread: Partial<Record<string, number>>;
};
export declare const notificationsSlice: Slice<NotificationsState, {
    notificationAdded: {
        reducer: (state: WritableDraft<NotificationsState>, action: PayloadAction<ProcessCompletedNotification>) => void;
        prepare: (event: ProcessCompletedEvent) => {
            payload: ProcessCompletedNotification;
        };
    };
    notificationSeen: {
        reducer: (state: WritableDraft<NotificationsState>, action: PayloadAction<{
            threadId: string;
            seenAt: number;
        }>) => void;
        prepare: (payload: {
            threadId: string;
        }) => {
            payload: {
                seenAt: number;
                threadId: string;
            };
        };
    };
    clearProcessCompletions: (state: WritableDraft<NotificationsState>) => void;
}, "notifications", "notifications", SliceSelectors<NotificationsState>>;
export declare const notificationAdded: ActionCreatorWithPreparedPayload<[event: {
    chat_id: string;
    seq: string;
    type: "process_completed";
    process_id: string;
    status: string;
    exit_code: number | null;
    short_description: string;
    mode: string;
}], ProcessCompletedNotification, "notifications/notificationAdded", never, never>, notificationSeen: ActionCreatorWithPreparedPayload<[payload: {
    threadId: string;
}], {
    seenAt: number;
    threadId: string;
}, "notifications/notificationSeen", never, never>, clearProcessCompletions: ActionCreatorWithoutPayload<"notifications/clearProcessCompletions">;
export declare const processCompleted: ActionCreatorWithPreparedPayload<[event: {
    chat_id: string;
    seq: string;
    type: "process_completed";
    process_id: string;
    status: string;
    exit_code: number | null;
    short_description: string;
    mode: string;
}], ProcessCompletedNotification, "notifications/notificationAdded", never, never>;
export declare const selectPendingNotificationsByThread: (state: {
    notifications: NotificationsState;
}) => Partial<Record<string, ProcessCompletedNotification[]>>;
export declare const selectLastSeenByThread: (state: {
    notifications: NotificationsState;
}) => Partial<Record<string, number>>;
export declare const selectProcessCompletions: ((state: {
    notifications: NotificationsState;
}) => ProcessCompletedNotification[]) & {
    clearCache: () => void;
    resultsCount: () => number;
    resetResultsCount: () => void;
} & {
    resultFunc: (resultFuncArgs_0: Partial<Record<string, ProcessCompletedNotification[]>>) => ProcessCompletedNotification[];
    memoizedResultFunc: ((resultFuncArgs_0: Partial<Record<string, ProcessCompletedNotification[]>>) => ProcessCompletedNotification[]) & {
        clearCache: () => void;
        resultsCount: () => number;
        resetResultsCount: () => void;
    };
    lastResult: () => ProcessCompletedNotification[];
    dependencies: [(state: {
        notifications: NotificationsState;
    }) => Partial<Record<string, ProcessCompletedNotification[]>>];
    recomputations: () => number;
    resetRecomputations: () => void;
    dependencyRecomputations: () => number;
    resetDependencyRecomputations: () => void;
} & {
    memoize: typeof weakMapMemoize;
    argsMemoize: typeof weakMapMemoize;
};
export declare const selectUnreadNotificationCountByThread: (state: {
    notifications: NotificationsState;
}, threadId: string) => number;
