import {
  createSelector,
  createSlice,
  type PayloadAction,
} from "@reduxjs/toolkit";
import type { ChatEventEnvelope } from "../../services/refact/chatSubscription";

export type ProcessCompletedEvent = Extract<
  ChatEventEnvelope,
  { type: "process_completed" }
>;

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
  silent?: boolean;
};

export type RecoveredProcessCompletion = {
  processId: string;
  status: string;
  exitCode: number | null;
  shortDescription: string;
  mode: string;
  seq?: string;
};

export type NotificationsState = {
  pendingByThread: Partial<Record<string, ProcessCompletedNotification[]>>;
  lastSeenByThread: Partial<Record<string, number>>;
};

const MAX_PENDING_PER_THREAD = 50;

const initialState: NotificationsState = {
  pendingByThread: {},
  lastSeenByThread: {},
};

function notificationId(event: ProcessCompletedEvent): string {
  return `${event.chat_id}:${event.process_id}:${event.seq}`;
}

function makeProcessCompletedNotification(
  event: ProcessCompletedEvent,
  receivedAt: number,
): ProcessCompletedNotification {
  return {
    id: notificationId(event),
    threadId: event.chat_id,
    seq: event.seq,
    processId: event.process_id,
    status: event.status,
    exitCode: event.exit_code,
    shortDescription: event.short_description,
    mode: event.mode,
    receivedAt,
  };
}

function markThreadSeen(
  state: NotificationsState,
  threadId: string,
  seenAt: number,
) {
  state.lastSeenByThread[threadId] = seenAt;
  state.pendingByThread[threadId] = undefined;
}

type ThreadVisitAction = PayloadAction<{ id: string }>;

function isThreadVisitAction(action: {
  type: string;
  payload?: unknown;
}): action is ThreadVisitAction {
  if (
    action.type !== "chatThread/switchToThread" &&
    action.type !== "chatThread/restoreChat"
  ) {
    return false;
  }
  const payload = action.payload;
  return (
    typeof payload === "object" &&
    payload !== null &&
    "id" in payload &&
    typeof (payload as { id?: unknown }).id === "string"
  );
}

export const notificationsSlice = createSlice({
  name: "notifications",
  initialState,
  reducers: {
    notificationAdded: {
      reducer: (state, action: PayloadAction<ProcessCompletedNotification>) => {
        const notification = action.payload;
        const lastSeen = state.lastSeenByThread[notification.threadId] ?? 0;
        if (notification.receivedAt <= lastSeen) return;

        const pending = state.pendingByThread[notification.threadId] ?? [];
        const existingIndex = pending.findIndex(
          (item) => item.processId === notification.processId,
        );
        if (existingIndex >= 0) {
          pending[existingIndex] = notification;
        } else {
          pending.push(notification);
        }

        if (pending.length > MAX_PENDING_PER_THREAD) {
          pending.splice(0, pending.length - MAX_PENDING_PER_THREAD);
        }

        state.pendingByThread[notification.threadId] = pending;
      },
      prepare: (event: ProcessCompletedEvent) => ({
        payload: makeProcessCompletedNotification(event, Date.now()),
      }),
    },
    notificationSeen: {
      reducer: (
        state,
        action: PayloadAction<{
          threadId: string;
          seenAt: number;
        }>,
      ) => {
        markThreadSeen(state, action.payload.threadId, action.payload.seenAt);
      },
      prepare: (payload: { threadId: string }) => ({
        payload: { ...payload, seenAt: Date.now() },
      }),
    },
    clearProcessCompletions: (state) => {
      state.pendingByThread = {};
      state.lastSeenByThread = {};
    },
    processCompletionsRecovered: (
      state,
      action: PayloadAction<{
        threadId: string;
        completions: RecoveredProcessCompletion[];
      }>,
    ) => {
      const { threadId, completions } = action.payload;
      if (completions.length === 0) return;
      const pending = state.pendingByThread[threadId] ?? [];
      const receivedAt = Date.now();
      for (const completion of completions) {
        if (pending.some((item) => item.processId === completion.processId)) {
          continue;
        }
        pending.push({
          id: `${threadId}:${completion.processId}`,
          threadId,
          seq: completion.seq ?? "0",
          processId: completion.processId,
          status: completion.status,
          exitCode: completion.exitCode,
          shortDescription: completion.shortDescription,
          mode: completion.mode,
          receivedAt,
          silent: true,
        });
      }
      if (pending.length > MAX_PENDING_PER_THREAD) {
        pending.splice(0, pending.length - MAX_PENDING_PER_THREAD);
      }
      state.pendingByThread[threadId] = pending;
    },
  },
  extraReducers: (builder) => {
    builder.addMatcher(isThreadVisitAction, (state, action) => {
      markThreadSeen(state, action.payload.id, Date.now());
    });
  },
});

export const {
  notificationAdded,
  notificationSeen,
  clearProcessCompletions,
  processCompletionsRecovered,
} = notificationsSlice.actions;
export const processCompleted = notificationAdded;

export const selectPendingNotificationsByThread = (state: {
  notifications: NotificationsState;
}) => state.notifications.pendingByThread;

export const selectLastSeenByThread = (state: {
  notifications: NotificationsState;
}) => state.notifications.lastSeenByThread;

export const selectProcessCompletions = createSelector(
  selectPendingNotificationsByThread,
  (pendingByThread): ProcessCompletedNotification[] => {
    const notifications: ProcessCompletedNotification[] = [];
    for (const pending of Object.values(pendingByThread)) {
      if (!pending) continue;
      for (const notification of pending) {
        if (!notification.silent) notifications.push(notification);
      }
    }
    return notifications;
  },
);

export const selectUnreadNotificationCountByThread = (
  state: { notifications: NotificationsState },
  threadId: string,
) => state.notifications.pendingByThread[threadId]?.length ?? 0;
