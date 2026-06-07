import { Reducer } from 'redux';
import { WritableDraft } from 'immer';
import { Slice, SliceSelectors, ActionCreatorWithPayload, ActionCreatorWithoutPayload, PayloadAction } from '@reduxjs/toolkit';
import { RootState } from "../../app/store";
export type BackendStatus = "unknown" | "online" | "offline";
export type SseStatus = "disconnected" | "connecting" | "connected";
export type SseConnectionInfo = {
    status: SseStatus;
    lastEventAt: number | null;
    retryCount: number;
    error: string | null;
};
export type ConnectionState = {
    browserOnline: boolean;
    backendStatus: BackendStatus;
    backendLastOkAt: number | null;
    backendError: string | null;
    sseConnections: Partial<Record<string, SseConnectionInfo>>;
};
export declare const connectionSlice: Slice<ConnectionState, {
    setBrowserOnline: (state: WritableDraft<ConnectionState>, action: PayloadAction<boolean>) => void;
    setBackendStatus: (state: WritableDraft<ConnectionState>, action: PayloadAction<{
        status: BackendStatus;
        error?: string | null;
    }>) => void;
    setSseStatus: (state: WritableDraft<ConnectionState>, action: PayloadAction<{
        chatId: string;
        status: SseStatus;
        error?: string | null;
    }>) => void;
    sseEventReceived: (state: WritableDraft<ConnectionState>, action: PayloadAction<{
        chatId: string;
    }>) => void;
    resetSseRetryCount: (state: WritableDraft<ConnectionState>, action: PayloadAction<{
        chatId: string;
    }>) => void;
    removeSseConnection: (state: WritableDraft<ConnectionState>, action: PayloadAction<{
        chatId: string;
    }>) => void;
    clearAllSseConnections: (state: WritableDraft<ConnectionState>) => void;
}, "connection", "connection", SliceSelectors<ConnectionState>>;
export declare const setBrowserOnline: ActionCreatorWithPayload<boolean, "connection/setBrowserOnline">, setBackendStatus: ActionCreatorWithPayload<{
    status: BackendStatus;
    error?: string | null;
}, "connection/setBackendStatus">, setSseStatus: ActionCreatorWithPayload<{
    chatId: string;
    status: SseStatus;
    error?: string | null;
}, "connection/setSseStatus">, sseEventReceived: ActionCreatorWithPayload<{
    chatId: string;
}, "connection/sseEventReceived">, resetSseRetryCount: ActionCreatorWithPayload<{
    chatId: string;
}, "connection/resetSseRetryCount">, removeSseConnection: ActionCreatorWithPayload<{
    chatId: string;
}, "connection/removeSseConnection">, clearAllSseConnections: ActionCreatorWithoutPayload<"connection/clearAllSseConnections">;
export declare const selectBrowserOnline: (state: RootState) => boolean;
export declare const selectBackendStatus: (state: RootState) => BackendStatus;
export declare const selectBackendLastOkAt: (state: RootState) => number | null;
export declare const selectSseConnections: (state: RootState) => Partial<Record<string, SseConnectionInfo>>;
export declare const selectSseConnectionForChat: (state: RootState, chatId: string) => SseConnectionInfo | undefined;
export declare const selectSseStatusForChat: (state: RootState, chatId: string) => SseStatus | null;
export declare const selectCurrentChatSseStatus: (state: RootState) => SseStatus | null;
export declare const selectGlobalSseStatus: (state: RootState) => SseStatus;
export declare const selectIsFullyConnected: (state: RootState) => boolean;
export declare const selectConnectionProblem: (state: RootState) => string | null;
export declare const selectMaxRetryCount: (state: RootState) => number;
declare const _default: Reducer<ConnectionState>;
export default _default;
