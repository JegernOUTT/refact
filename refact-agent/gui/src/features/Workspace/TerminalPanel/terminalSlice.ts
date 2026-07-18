import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type { ExecStatus } from "../../../services/refact/exec";

export type TerminalSessionMetadata = {
  process_id: string;
  title: string;
  status: ExecStatus;
};

export type TerminalState = {
  sessions: TerminalSessionMetadata[];
  activeProcessId: string | null;
};

const initialState: TerminalState = {
  sessions: [],
  activeProcessId: null,
};

export const terminalSlice = createSlice({
  name: "terminal",
  reducerPath: "terminal",
  initialState,
  reducers: {
    sessionAdded: (state, action: PayloadAction<TerminalSessionMetadata>) => {
      const existing = state.sessions.find(
        (session) => session.process_id === action.payload.process_id,
      );
      if (existing) {
        Object.assign(existing, action.payload);
      } else {
        state.sessions.push(action.payload);
      }
      state.activeProcessId = action.payload.process_id;
    },
    sessionsReattached: (
      state,
      action: PayloadAction<TerminalSessionMetadata[]>,
    ) => {
      for (const session of action.payload) {
        if (
          !state.sessions.some((item) => item.process_id === session.process_id)
        ) {
          state.sessions.push(session);
        }
      }
      if (!state.activeProcessId && state.sessions.length > 0) {
        state.activeProcessId = state.sessions[0].process_id;
      }
    },
    activeSessionChanged: (state, action: PayloadAction<string>) => {
      if (
        state.sessions.some((session) => session.process_id === action.payload)
      ) {
        state.activeProcessId = action.payload;
      }
    },
    sessionStatusChanged: (
      state,
      action: PayloadAction<{ processId: string; status: ExecStatus }>,
    ) => {
      const session = state.sessions.find(
        (item) => item.process_id === action.payload.processId,
      );
      if (session) session.status = action.payload.status;
    },
    sessionRemoved: (state, action: PayloadAction<string>) => {
      const index = state.sessions.findIndex(
        (session) => session.process_id === action.payload,
      );
      if (index === -1) return;
      state.sessions.splice(index, 1);
      if (state.activeProcessId === action.payload) {
        state.activeProcessId =
          state.sessions.at(index)?.process_id ??
          state.sessions.at(index - 1)?.process_id ??
          null;
      }
    },
  },
});

export const {
  activeSessionChanged,
  sessionAdded,
  sessionRemoved,
  sessionsReattached,
  sessionStatusChanged,
} = terminalSlice.actions;

type TerminalRootState = {
  terminal: TerminalState;
};

export const selectTerminalSessions = (state: TerminalRootState) =>
  state.terminal.sessions;

export const selectActiveTerminalProcessId = (state: TerminalRootState) =>
  state.terminal.activeProcessId;

export default terminalSlice.reducer;
