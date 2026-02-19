import { createSlice, PayloadAction } from "@reduxjs/toolkit";
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

const initialState: ConnectionState = {
  browserOnline: typeof navigator !== "undefined" ? navigator.onLine : true,
  backendStatus: "unknown",
  backendLastOkAt: null,
  backendError: null,
  sseConnections: {},
};

export const connectionSlice = createSlice({
  name: "connection",
  initialState,
  reducers: {
    setBrowserOnline: (state, action: PayloadAction<boolean>) => {
      state.browserOnline = action.payload;
    },

    setBackendStatus: (
      state,
      action: PayloadAction<{
        status: BackendStatus;
        error?: string | null;
      }>,
    ) => {
      state.backendStatus = action.payload.status;
      if (action.payload.status === "online") {
        state.backendLastOkAt = Date.now();
        state.backendError = null;
      } else if (action.payload.error) {
        state.backendError = action.payload.error;
      }
    },

    setSseStatus: (
      state,
      action: PayloadAction<{
        chatId: string;
        status: SseStatus;
        error?: string | null;
      }>,
    ) => {
      const { chatId, status, error } = action.payload;
      const existing = state.sseConnections[chatId];

      if (!existing) {
        state.sseConnections[chatId] = {
          status,
          lastEventAt: status === "connected" ? Date.now() : null,
          retryCount: status === "disconnected" ? 1 : 0,
          error: error ?? null,
        };
      } else {
        existing.status = status;
        if (status === "connected") {
          existing.lastEventAt = Date.now();
          existing.retryCount = 0;
          existing.error = null;
        } else if (status === "disconnected") {
          existing.retryCount += 1;
          if (error) {
            existing.error = error;
          }
        }
      }
    },

    sseEventReceived: (state, action: PayloadAction<{ chatId: string }>) => {
      const conn = state.sseConnections[action.payload.chatId];
      if (conn) {
        conn.lastEventAt = Date.now();
      }
    },

    resetSseRetryCount: (state, action: PayloadAction<{ chatId: string }>) => {
      const conn = state.sseConnections[action.payload.chatId];
      if (conn) {
        conn.retryCount = 0;
      }
    },

    removeSseConnection: (state, action: PayloadAction<{ chatId: string }>) => {
      // eslint-disable-next-line @typescript-eslint/no-dynamic-delete
      delete state.sseConnections[action.payload.chatId];
    },

    clearAllSseConnections: (state) => {
      state.sseConnections = {};
    },
  },
});

export const {
  setBrowserOnline,
  setBackendStatus,
  setSseStatus,
  sseEventReceived,
  resetSseRetryCount,
  removeSseConnection,
  clearAllSseConnections,
} = connectionSlice.actions;

export const selectBrowserOnline = (state: RootState) =>
  state.connection.browserOnline;

export const selectBackendStatus = (state: RootState) =>
  state.connection.backendStatus;

export const selectBackendLastOkAt = (state: RootState) =>
  state.connection.backendLastOkAt;

export const selectSseConnections = (state: RootState) =>
  state.connection.sseConnections;

export const selectSseConnectionForChat = (state: RootState, chatId: string) =>
  state.connection.sseConnections[chatId];

export const selectSseStatusForChat = (state: RootState, chatId: string) =>
  state.connection.sseConnections[chatId]?.status ?? null;

export const selectCurrentChatSseStatus = (
  state: RootState,
): SseStatus | null => {
  const currentId = state.chat.current_thread_id;
  if (!currentId) return null;
  const conn = state.connection.sseConnections[currentId];
  return conn?.status ?? "disconnected";
};

export const selectGlobalSseStatus = (state: RootState): SseStatus => {
  const connections = Object.values(state.connection.sseConnections).filter(
    (c): c is SseConnectionInfo => c !== undefined,
  );
  if (connections.length === 0) return "disconnected";
  if (connections.some((c) => c.status === "connecting")) return "connecting";
  if (connections.every((c) => c.status === "connected")) return "connected";
  return "disconnected";
};

export const selectIsFullyConnected = (state: RootState): boolean => {
  if (!state.connection.browserOnline) return false;
  if (state.connection.backendStatus !== "online") return false;
  const sseStatus = selectCurrentChatSseStatus(state);
  if (sseStatus === null) return true;
  return sseStatus === "connected";
};

export const selectConnectionProblem = (state: RootState): string | null => {
  if (!state.connection.browserOnline) {
    return "Browser is offline";
  }
  if (state.connection.backendStatus === "offline") {
    return "Backend server unreachable";
  }
  if (state.connection.backendStatus === "unknown") {
    return "Connecting to backend...";
  }
  const currentSseStatus = selectCurrentChatSseStatus(state);
  if (currentSseStatus === null) {
    return null;
  }
  if (currentSseStatus === "disconnected") {
    return "Real-time connection lost";
  }
  if (currentSseStatus === "connecting") {
    return "Connecting...";
  }
  return null;
};

export const selectMaxRetryCount = (state: RootState): number => {
  const connections = Object.values(state.connection.sseConnections).filter(
    (c): c is SseConnectionInfo => c !== undefined,
  );
  if (connections.length === 0) return 0;
  return Math.max(...connections.map((c) => c.retryCount));
};

export default connectionSlice.reducer;
