import { useEffect, useRef, useCallback } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import {
  subscribeToTaskEvents,
  TaskEventEnvelope,
} from "../services/refact/tasksSubscription";
import { tasksApi } from "../services/refact/tasks";

export function useTasksSubscription() {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const disconnectRef = useRef<(() => void) | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const connect = useCallback(() => {
    if (disconnectRef.current) {
      disconnectRef.current();
      disconnectRef.current = null;
    }
    if (reconnectTimeoutRef.current) {
      clearTimeout(reconnectTimeoutRef.current);
      reconnectTimeoutRef.current = null;
    }

    const port = config.lspPort;
    const apiKey = config.apiKey ?? null;

    const onEvent = (envelope: TaskEventEnvelope) => {
      switch (envelope.type) {
        case "snapshot":
          dispatch(
            tasksApi.util.updateQueryData("listTasks", undefined, () => envelope.tasks),
          );
          break;

        case "task_created":
          dispatch(
            tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
              const exists = draft.some((t) => t.id === envelope.task_id);
              if (!exists) {
                draft.unshift(envelope.meta);
              }
            }),
          );
          break;

        case "task_updated":
          dispatch(
            tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
              const index = draft.findIndex((t) => t.id === envelope.task_id);
              if (index >= 0) {
                const existing = draft[index];
                draft[index] = {
                  ...envelope.meta,
                  planner_session_state: envelope.meta.planner_session_state ?? existing.planner_session_state,
                };
              }
              draft.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
            }),
          );
          dispatch(
            tasksApi.util.updateQueryData("getTask", envelope.task_id, (existing) => ({
              ...envelope.meta,
              planner_session_state: envelope.meta.planner_session_state ?? existing?.planner_session_state,
            })),
          );
          break;

        case "task_deleted":
          dispatch(
            tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
              const index = draft.findIndex((t) => t.id === envelope.task_id);
              if (index >= 0) {
                draft.splice(index, 1);
              }
            }),
          );
          break;

        case "board_changed":
          dispatch(
            tasksApi.util.updateQueryData("getBoard", envelope.task_id, () => envelope.board),
          );
          break;
      }
    };

    const scheduleReconnect = () => {
      reconnectTimeoutRef.current = setTimeout(connect, 5000);
    };

    disconnectRef.current = subscribeToTaskEvents(port, apiKey, {
      onEvent,
      onError: scheduleReconnect,
      onDisconnected: scheduleReconnect,
    });
  }, [dispatch, config.lspPort, config.apiKey]);

  useEffect(() => {
    connect();
    return () => {
      if (disconnectRef.current) {
        disconnectRef.current();
      }
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
    };
  }, [connect]);
}
