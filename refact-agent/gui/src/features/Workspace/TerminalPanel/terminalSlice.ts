import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type { ExecStatus } from "../../../services/refact/exec";

export const MAX_RECENTLY_CLOSED_PROCESSES = 256;

export function isFinishedStatus(status: ExecStatus): boolean {
  return status !== "starting" && status !== "running";
}

export type TerminalSessionMetadata = {
  process_id: string;
  label: string;
  title: string;
  status: ExecStatus;
  tty?: boolean;
};

export function terminalSessionFromProcess({
  process_id: processId,
  command_preview: commandPreview,
  status,
  tty,
}: {
  process_id: string;
  command_preview?: string;
  status: ExecStatus;
  tty: boolean;
}): TerminalSessionMetadata {
  const preview = commandPreview?.trim();
  const label = preview && preview.length > 0 ? preview : "shell";
  return {
    process_id: processId,
    label,
    title: `${label} · ${processId.slice(0, 8)}`,
    status,
    tty,
  };
}

export type TerminalState = {
  sessionsByChat: Record<string, TerminalSessionMetadata[] | undefined>;
  activeProcessIdByChat: Record<string, string | null | undefined>;
  workbenchOpenByChat: Record<string, boolean | undefined>;
  recentlyClosedByChat: Record<string, string[] | undefined>;
};

const initialState: TerminalState = {
  sessionsByChat: {},
  activeProcessIdByChat: {},
  workbenchOpenByChat: {},
  recentlyClosedByChat: {},
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

function closeSession(state: TerminalState, chatId: string, processId: string) {
  const closed = (state.recentlyClosedByChat[chatId] ??= []);
  if (!closed.includes(processId)) {
    closed.push(processId);
    if (closed.length > MAX_RECENTLY_CLOSED_PROCESSES) closed.shift();
  }

  const sessions = state.sessionsByChat[chatId];
  if (!sessions) return;
  const index = sessions.findIndex(
    (session) => session.process_id === processId,
  );
  if (index < 0) return;
  sessions.splice(index, 1);
  if (state.activeProcessIdByChat[chatId] === processId) {
    state.activeProcessIdByChat[chatId] =
      sessions.at(index)?.process_id ??
      sessions.at(index - 1)?.process_id ??
      null;
  }
}

export const terminalSlice = createSlice({
  name: "terminal",
  reducerPath: "terminal",
  initialState,
  reducers: {
    sessionAdded: (state, action: PayloadAction<ChatSessionPayload>) => {
      const { chatId, session } = action.payload;
      if (isFinishedStatus(session.status)) {
        closeSession(state, chatId, session.process_id);
        return;
      }
      if (state.recentlyClosedByChat[chatId]?.includes(session.process_id)) {
        return;
      }
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
      const { chatId, sessions } = action.payload;
      const knownIds = new Set(sessions.map((session) => session.process_id));
      const closedIds = new Set(state.recentlyClosedByChat[chatId]);
      const finishedIds = new Set(
        sessions
          .filter((session) => isFinishedStatus(session.status))
          .map((session) => session.process_id),
      );
      for (const session of (state.sessionsByChat[chatId] ?? []).slice()) {
        if (finishedIds.has(session.process_id))
          closeSession(state, chatId, session.process_id);
      }
      const reattachedSessions = [
        ...sessions.filter(
          (session) =>
            !isFinishedStatus(session.status) &&
            !closedIds.has(session.process_id),
        ),
        ...(state.sessionsByChat[chatId] ?? []).filter(
          (session) => !knownIds.has(session.process_id),
        ),
      ];
      const activeProcessId = state.activeProcessIdByChat[chatId];
      state.sessionsByChat[chatId] = reattachedSessions;
      state.activeProcessIdByChat[chatId] =
        activeProcessId &&
        reattachedSessions.some(
          (session) => session.process_id === activeProcessId,
        )
          ? activeProcessId
          : reattachedSessions[0]?.process_id ?? null;
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
      const { chatId, processId, status } = action.payload;
      if (isFinishedStatus(status)) {
        closeSession(state, chatId, processId);
        return;
      }
      const session = state.sessionsByChat[chatId]?.find(
        (item) => item.process_id === processId,
      );
      if (session) session.status = status;
    },
    sessionRemoved: (state, action: PayloadAction<ChatProcessPayload>) => {
      const { chatId, processId } = action.payload;
      closeSession(state, chatId, processId);
    },
    completedSessionsReconciled: (
      state,
      action: PayloadAction<{ chatId: string; processIds: string[] }>,
    ) => {
      const { chatId, processIds } = action.payload;
      for (const processId of processIds)
        closeSession(state, chatId, processId);
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
    clearTerminalChatState: (state, action: PayloadAction<string>) => {
      const { [action.payload]: _sessions, ...otherSessions } =
        state.sessionsByChat;
      const { [action.payload]: _activeProcessId, ...otherActiveProcessIds } =
        state.activeProcessIdByChat;
      const { [action.payload]: _workbenchOpen, ...otherWorkbenchOpen } =
        state.workbenchOpenByChat;
      const { [action.payload]: _closed, ...otherClosed } =
        state.recentlyClosedByChat;
      state.sessionsByChat = otherSessions;
      state.activeProcessIdByChat = otherActiveProcessIds;
      state.workbenchOpenByChat = otherWorkbenchOpen;
      state.recentlyClosedByChat = otherClosed;
    },
  },
});

export const {
  activeSessionChanged,
  clearTerminalChatState,
  completedSessionsReconciled,
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
