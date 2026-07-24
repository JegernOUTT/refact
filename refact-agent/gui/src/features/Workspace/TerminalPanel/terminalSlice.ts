import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type { ExecStatus } from "../../../services/refact/exec";

export type TerminalSessionMetadata = {
  process_id: string;
  title: string;
  status: ExecStatus;
};

export type TerminalState = {
  sessionsByChat: Record<string, TerminalSessionMetadata[] | undefined>;
  activeProcessIdByChat: Record<string, string | null | undefined>;
  workbenchOpenByChat: Record<string, boolean | undefined>;
};

const initialState: TerminalState = {
  sessionsByChat: {},
  activeProcessIdByChat: {},
  workbenchOpenByChat: {},
};

type ChatSessionPayload = {
  chatId: string;
  session: TerminalSessionMetadata;
};

type ChatSessionsPayload = {
  chatId: string;
  sessions: TerminalSessionMetadata[];
};

type ChatProcessPayload = {
  chatId: string;
  processId: string;
};

export const terminalSlice = createSlice({
  name: "terminal",
  reducerPath: "terminal",
  initialState,
  reducers: {
    sessionAdded: (state, action: PayloadAction<ChatSessionPayload>) => {
      const { chatId, session } = action.payload;
      const sessions = (state.sessionsByChat[chatId] ??= []);
      const existing = sessions.find(
        (item) => item.process_id === session.process_id,
      );
      if (existing) {
        Object.assign(existing, session);
      } else {
        sessions.push(session);
      }
      state.activeProcessIdByChat[chatId] = session.process_id;
    },
    sessionsReattached: (state, action: PayloadAction<ChatSessionsPayload>) => {
      const { chatId, sessions: reattachedSessions } = action.payload;
      const sessions = (state.sessionsByChat[chatId] ??= []);
      for (const session of reattachedSessions) {
        const existing = sessions.find(
          (item) => item.process_id === session.process_id,
        );
        if (existing) {
          Object.assign(existing, session);
        } else {
          sessions.push(session);
        }
      }
      if (!state.activeProcessIdByChat[chatId] && sessions.length > 0) {
        state.activeProcessIdByChat[chatId] = sessions[0].process_id;
      }
    },
    activeSessionChanged: (
      state,
      action: PayloadAction<ChatProcessPayload>,
    ) => {
      const { chatId, processId } = action.payload;
      if (
        state.sessionsByChat[chatId]?.some(
          (session) => session.process_id === processId,
        )
      ) {
        state.activeProcessIdByChat[chatId] = processId;
      }
    },
    sessionStatusChanged: (
      state,
      action: PayloadAction<ChatProcessPayload & { status: ExecStatus }>,
    ) => {
      const session = state.sessionsByChat[action.payload.chatId]?.find(
        (item) => item.process_id === action.payload.processId,
      );
      if (session) session.status = action.payload.status;
    },
    sessionRemoved: (state, action: PayloadAction<ChatProcessPayload>) => {
      const { chatId, processId } = action.payload;
      const sessions = state.sessionsByChat[chatId];
      if (!sessions) return;
      const index = sessions.findIndex(
        (session) => session.process_id === processId,
      );
      if (index === -1) return;
      sessions.splice(index, 1);
      if (state.activeProcessIdByChat[chatId] === processId) {
        state.activeProcessIdByChat[chatId] =
          sessions.at(index)?.process_id ??
          sessions.at(index - 1)?.process_id ??
          null;
      }
    },
    setTerminalWorkbenchOpen: (
      state,
      action: PayloadAction<{ chatId: string; open: boolean }>,
    ) => {
      state.workbenchOpenByChat[action.payload.chatId] = action.payload.open;
    },
    toggleTerminalWorkbench: (
      state,
      action: PayloadAction<{ chatId: string }>,
    ) => {
      const { chatId } = action.payload;
      state.workbenchOpenByChat[chatId] = !(
        state.workbenchOpenByChat[chatId] ?? false
      );
    },
  },
});

export const {
  activeSessionChanged,
  sessionAdded,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
  setTerminalWorkbenchOpen,
  toggleTerminalWorkbench,
} = terminalSlice.actions;

type TerminalRootState = {
  terminal: TerminalState;
};

const EMPTY_TERMINAL_SESSIONS: TerminalSessionMetadata[] = [];

export const selectTerminalSessions = (
  state: TerminalRootState,
  chatId: string | null,
) =>
  chatId
    ? state.terminal.sessionsByChat[chatId] ?? EMPTY_TERMINAL_SESSIONS
    : EMPTY_TERMINAL_SESSIONS;

export const selectActiveTerminalProcessId = (
  state: TerminalRootState,
  chatId: string | null,
) => (chatId ? state.terminal.activeProcessIdByChat[chatId] ?? null : null);

export const selectTerminalWorkbenchOpen = (
  state: TerminalRootState,
  chatId: string | null,
) => (chatId ? state.terminal.workbenchOpenByChat[chatId] ?? false : false);

export default terminalSlice.reducer;
