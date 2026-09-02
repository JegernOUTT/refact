import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import type { ExecStatus } from "../../../services/refact/exec";

export const FINISHED_SESSION_TTL_MS = 10 * 60_000;

export function isFinishedStatus(status: ExecStatus): boolean {
  return status !== "starting" && status !== "running";
}

export type TerminalSessionMetadata = {
  process_id: string;
  label: string;
  title: string;
  status: ExecStatus;
  tty?: boolean;
  exit_code?: number | null;
  ended_at_ms?: number | null;
};

export function terminalSessionFromProcess({
  process_id: processId,
  command_preview: commandPreview,
  status,
  tty,
  exit_code: exitCode,
  ended_at_ms: endedAtMs,
}: {
  process_id: string;
  command_preview?: string;
  status: ExecStatus;
  tty: boolean;
  exit_code?: number | null;
  ended_at_ms?: number | null;
}): TerminalSessionMetadata {
  const preview = commandPreview?.trim();
  const label = preview && preview.length > 0 ? preview : "shell";
  return {
    process_id: processId,
    label,
    title: `${label} · ${processId.slice(0, 8)}`,
    status,
    tty,
    ...(exitCode === undefined ? {} : { exit_code: exitCode }),
    ...(endedAtMs === undefined ? {} : { ended_at_ms: endedAtMs }),
  };
}

export function sessionExpiresAt(
  session: TerminalSessionMetadata,
): number | null {
  if (!isFinishedStatus(session.status)) return null;
  if (typeof session.ended_at_ms !== "number") return null;
  return session.ended_at_ms + FINISHED_SESSION_TTL_MS;
}

function nextActiveProcessId(
  sessions: TerminalSessionMetadata[],
  removedIndex: number,
): string | null {
  return (
    sessions.at(removedIndex)?.process_id ??
    sessions.at(removedIndex - 1)?.process_id ??
    null
  );
}

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
      action: PayloadAction<
        ChatProcessPayload & {
          status: ExecStatus;
          exit_code?: number | null;
          ended_at_ms?: number | null;
        }
      >,
    ) => {
      const session = state.sessionsByChat[action.payload.chatId]?.find(
        (item) => item.process_id === action.payload.processId,
      );
      if (session) {
        session.status = action.payload.status;
        if (action.payload.exit_code !== undefined) {
          session.exit_code = action.payload.exit_code;
        }
        if (action.payload.ended_at_ms !== undefined) {
          session.ended_at_ms = action.payload.ended_at_ms;
        }
      }
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
        state.activeProcessIdByChat[chatId] = nextActiveProcessId(
          sessions,
          index,
        );
      }
    },
    expiredSessionsPruned: (
      state,
      action: PayloadAction<{
        chatId: string;
        now: number;
        keepProcessId?: string | null;
      }>,
    ) => {
      const { chatId, now, keepProcessId } = action.payload;
      const sessions = state.sessionsByChat[chatId];
      if (!sessions) return;
      for (let index = sessions.length - 1; index >= 0; index -= 1) {
        const session = sessions[index];
        const expiresAt = sessionExpiresAt(session);
        if (
          expiresAt === null ||
          expiresAt > now ||
          session.process_id === keepProcessId
        ) {
          continue;
        }
        sessions.splice(index, 1);
        if (state.activeProcessIdByChat[chatId] === session.process_id) {
          state.activeProcessIdByChat[chatId] = nextActiveProcessId(
            sessions,
            index,
          );
        }
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
    clearTerminalChatState: (state, action: PayloadAction<string>) => {
      const { [action.payload]: _sessions, ...otherSessions } =
        state.sessionsByChat;
      const { [action.payload]: _activeProcessId, ...otherActiveProcessIds } =
        state.activeProcessIdByChat;
      const { [action.payload]: _workbenchOpen, ...otherWorkbenchOpen } =
        state.workbenchOpenByChat;
      state.sessionsByChat = otherSessions;
      state.activeProcessIdByChat = otherActiveProcessIds;
      state.workbenchOpenByChat = otherWorkbenchOpen;
    },
  },
});

export const {
  activeSessionChanged,
  clearTerminalChatState,
  expiredSessionsPruned,
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
