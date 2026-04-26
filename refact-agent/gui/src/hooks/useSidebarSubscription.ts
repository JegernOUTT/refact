import { useEffect, useRef, useCallback } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import {
  subscribeToSidebarEvents,
  SidebarEventEnvelope,
} from "../services/refact/sidebarSubscription";
import type { TrajectoryMeta } from "../services/refact/trajectories";
import {
  hydrateHistoryFromMeta,
  replaceSnapshotHistory,
  deleteChatById,
  updateChatMetaById,
  setHistoryLoading,
  setHistoryLoadError,
  setPagination,
} from "../features/History/historySlice";
import type { ChatHistoryItem } from "../features/History/historySlice";
import {
  updateOpenThread,
  closeThread,
  updateChatRuntimeFromSessionState,
} from "../features/Chat/Thread";
import { tasksApi } from "../services/refact/tasks";
import {
  trajectoriesApi,
  chatThreadToTrajectoryData,
} from "../services/refact/trajectories";
import { useAppSelector } from "./useAppSelector";

const RECONNECT_DELAY_MS = 500;
const MIGRATION_KEY = "refact-trajectories-migrated";

function getLegacyHistory(): ChatHistoryItem[] {
  try {
    const raw = localStorage.getItem("persist:root");
    if (!raw) return [];

    const parsed = JSON.parse(raw) as Record<string, string>;
    if (!parsed.history) return [];

    const historyData = JSON.parse(parsed.history) as unknown;
    if (typeof historyData !== "object" || historyData === null) return [];

    const historyObj = historyData as Record<string, unknown>;
    const chats =
      "chats" in historyObj && typeof historyObj.chats === "object"
        ? (historyObj.chats as Record<string, ChatHistoryItem>)
        : (historyObj as Record<string, ChatHistoryItem>);

    const values = Object.values(chats) as unknown[];
    return values.filter((item): item is ChatHistoryItem => {
      if (typeof item !== "object" || item === null) return false;
      const obj = item as Record<string, unknown>;
      return "id" in obj && "messages" in obj && Array.isArray(obj.messages);
    });
  } catch {
    return [];
  }
}

function clearLegacyHistory() {
  try {
    const raw = localStorage.getItem("persist:root");
    if (!raw) return;

    const parsed = JSON.parse(raw) as Record<string, string>;
    parsed.history = "{}";
    localStorage.setItem("persist:root", JSON.stringify(parsed));
  } catch {
    // Ignore localStorage errors
  }
}

function isMigrationDone(): boolean {
  return localStorage.getItem(MIGRATION_KEY) === "true";
}

function markMigrationDone() {
  localStorage.setItem(MIGRATION_KEY, "true");
}

export function useSidebarSubscription() {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const historyChats = useAppSelector((state) => state.history.chats);
  const historyRef = useRef(historyChats);
  historyRef.current = historyChats;
  const disconnectRef = useRef<(() => void) | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const initialLoadDoneRef = useRef(false);
  // eslint-disable-next-line @typescript-eslint/no-empty-function
  const connectRef = useRef<() => void>(() => {});

  const processTrajectoryEvent = useCallback(
    (event: SidebarEventEnvelope & { category: "trajectory" }) => {
      if (event.type === "deleted") {
        dispatch(deleteChatById(event.id));
        dispatch(closeThread({ id: event.id, force: true }));
        return;
      }

      const existsInHistory = event.id in historyRef.current;
      const hasMetaUpdate =
        event.title !== undefined ||
        event.updated_at !== undefined ||
        event.session_state !== undefined ||
        event.message_count !== undefined ||
        event.parent_id !== undefined ||
        event.link_type !== undefined ||
        event.root_chat_id !== undefined ||
        event.is_title_generated !== undefined ||
        event.error !== undefined ||
        event.model !== undefined ||
        event.mode !== undefined ||
        event.total_coins !== undefined ||
        event.total_lines_added !== undefined ||
        event.total_lines_removed !== undefined ||
        event.tasks_total !== undefined ||
        event.tasks_done !== undefined ||
        event.tasks_failed !== undefined;

      if (existsInHistory && hasMetaUpdate) {
        const metaPatch: Record<string, unknown> = { id: event.id };
        if (event.title !== undefined) metaPatch.title = event.title;
        if (event.is_title_generated !== undefined)
          metaPatch.isTitleGenerated = event.is_title_generated;
        if (event.updated_at !== undefined)
          metaPatch.updatedAt = event.updated_at;
        if (event.session_state !== undefined)
          metaPatch.session_state = event.session_state;
        if (event.message_count !== undefined)
          metaPatch.message_count = event.message_count;
        if (event.parent_id !== undefined)
          metaPatch.parent_id = event.parent_id;
        if (event.link_type !== undefined)
          metaPatch.link_type = event.link_type;
        if (event.root_chat_id !== undefined)
          metaPatch.root_chat_id = event.root_chat_id;
        if (event.total_coins !== undefined)
          metaPatch.total_coins = event.total_coins;
        if (event.total_lines_added !== undefined)
          metaPatch.total_lines_added = event.total_lines_added;
        if (event.total_lines_removed !== undefined)
          metaPatch.total_lines_removed = event.total_lines_removed;
        if (event.model !== undefined) metaPatch.model = event.model;
        if (event.mode !== undefined) metaPatch.mode = event.mode;
        if (event.tasks_total !== undefined)
          metaPatch.tasks_total = event.tasks_total;
        if (event.tasks_done !== undefined)
          metaPatch.tasks_done = event.tasks_done;
        if (event.tasks_failed !== undefined)
          metaPatch.tasks_failed = event.tasks_failed;
        dispatch(
          updateChatMetaById(
            metaPatch as Parameters<typeof updateChatMetaById>[0],
          ),
        );

        if (
          event.title !== undefined ||
          event.is_title_generated !== undefined
        ) {
          const threadPatch: Record<string, unknown> = {};
          if (event.title !== undefined) threadPatch.title = event.title;
          if (event.is_title_generated !== undefined)
            threadPatch.isTitleGenerated = event.is_title_generated;
          if (Object.keys(threadPatch).length > 0) {
            dispatch(
              updateOpenThread({
                id: event.id,
                thread: threadPatch as Parameters<
                  typeof updateOpenThread
                >[0]["thread"],
              }),
            );
          }
        }
        if (event.session_state !== undefined) {
          dispatch(
            updateChatRuntimeFromSessionState({
              id: event.id,
              session_state: event.session_state,
              error: event.error,
            }),
          );
        }
      } else if (
        !existsInHistory &&
        (event.title !== undefined || event.session_state !== undefined) &&
        event.updated_at
      ) {
        dispatch(
          hydrateHistoryFromMeta([
            {
              id: event.id,
              title: event.title ?? "New Chat",
              created_at: event.updated_at,
              updated_at: event.updated_at,
              model: event.model ?? "",
              mode: event.mode ?? "AGENT",
              message_count: event.message_count ?? 0,
              session_state: event.session_state,
              parent_id: event.parent_id,
              link_type: event.link_type,
              root_chat_id: event.root_chat_id,
              total_coins: event.total_coins,
              total_lines_added: event.total_lines_added ?? 0,
              total_lines_removed: event.total_lines_removed ?? 0,
              tasks_total: 0,
              tasks_done: 0,
              tasks_failed: 0,
            },
          ]),
        );
        const threadPatch: Record<string, unknown> = {};
        if (event.title !== undefined) threadPatch.title = event.title;
        if (event.is_title_generated !== undefined)
          threadPatch.isTitleGenerated = event.is_title_generated;
        if (Object.keys(threadPatch).length > 0) {
          dispatch(
            updateOpenThread({
              id: event.id,
              thread: threadPatch as Parameters<
                typeof updateOpenThread
              >[0]["thread"],
            }),
          );
        }
        if (event.session_state !== undefined) {
          dispatch(
            updateChatRuntimeFromSessionState({
              id: event.id,
              session_state: event.session_state,
              error: event.error,
            }),
          );
        }
      }
    },
    [dispatch],
  );

  const processTaskEvent = useCallback(
    (event: SidebarEventEnvelope & { category: "task" }) => {
      switch (event.type) {
        case "snapshot":
          dispatch(
            tasksApi.util.updateQueryData(
              "listTasks",
              undefined,
              () => event.tasks,
            ),
          );
          break;

        case "task_created":
          dispatch(
            tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
              const exists = draft.some((t) => t.id === event.task_id);
              if (!exists) {
                draft.unshift(event.meta);
              }
            }),
          );
          break;

        case "task_updated":
          dispatch(
            tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
              const index = draft.findIndex((t) => t.id === event.task_id);
              if (index >= 0) {
                const existing = draft[index];
                draft[index] = {
                  ...event.meta,
                  planner_session_state:
                    event.meta.planner_session_state ??
                    existing.planner_session_state,
                };
              }
              draft.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
            }),
          );
          dispatch(
            tasksApi.util.updateQueryData(
              "getTask",
              event.task_id,
              (existing) => ({
                ...event.meta,
                planner_session_state:
                  event.meta.planner_session_state ??
                  existing.planner_session_state,
              }),
            ),
          );
          break;

        case "task_deleted":
          dispatch(
            tasksApi.util.updateQueryData("listTasks", undefined, (draft) => {
              const index = draft.findIndex((t) => t.id === event.task_id);
              if (index >= 0) {
                draft.splice(index, 1);
              }
            }),
          );
          break;

        case "board_changed":
          dispatch(
            tasksApi.util.updateQueryData(
              "getBoard",
              event.task_id,
              () => event.board,
            ),
          );
          break;
      }
    },
    [dispatch],
  );

  const processSnapshot = useCallback(
    (event: SidebarEventEnvelope & { category: "snapshot" }) => {
      const trajectoryItems = event.trajectories.map((t: TrajectoryMeta) => ({
        id: t.id,
        title: t.title,
        created_at: t.created_at,
        updated_at: t.updated_at,
        model: t.model,
        mode: t.mode,
        message_count: t.message_count,
        session_state: t.session_state,
        parent_id: t.parent_id,
        link_type: t.link_type,
        root_chat_id: t.root_chat_id,
        total_coins: t.total_coins,
        total_lines_added: t.total_lines_added,
        total_lines_removed: t.total_lines_removed,
        tasks_total: t.tasks_total,
        tasks_done: t.tasks_done,
        tasks_failed: t.tasks_failed,
      }));

      dispatch(replaceSnapshotHistory(trajectoryItems));
      dispatch(setHistoryLoadError(null));
      dispatch(setHistoryLoading(false));

      dispatch(
        tasksApi.util.updateQueryData(
          "listTasks",
          undefined,
          () => event.tasks,
        ),
      );
    },
    [dispatch],
  );

  const migrateFromLocalStorage = useCallback(async () => {
    if (isMigrationDone()) return;

    const legacyChats = getLegacyHistory();
    if (legacyChats.length === 0) {
      markMigrationDone();
      return;
    }

    let successCount = 0;
    for (const chat of legacyChats) {
      if (chat.messages.length === 0) continue;

      try {
        const trajectoryData = chatThreadToTrajectoryData(
          {
            ...chat,
            new_chat_suggested: chat.new_chat_suggested ?? {
              wasSuggested: false,
            },
          },
          chat.createdAt,
        );
        trajectoryData.updated_at = chat.updatedAt;

        await dispatch(
          trajectoriesApi.endpoints.saveTrajectory.initiate(trajectoryData),
        ).unwrap();
        successCount++;
      } catch {
        // Ignore individual chat migration failures
      }
    }

    if (successCount > 0) {
      clearLegacyHistory();
    }
    markMigrationDone();
  }, [dispatch]);

  const loadInitialHistory = useCallback(async () => {
    dispatch(setHistoryLoading(true));
    try {
      await migrateFromLocalStorage();

      const result = await dispatch(
        trajectoriesApi.endpoints.listTrajectoriesPaginated.initiate(
          { limit: 50 },
          { forceRefetch: true },
        ),
      ).unwrap();

      dispatch(hydrateHistoryFromMeta(result.items));
      dispatch(
        setPagination({
          cursor: result.next_cursor,
          hasMore: result.has_more,
        }),
      );
      dispatch(setHistoryLoading(false));
      initialLoadDoneRef.current = true;
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to load history";
      dispatch(setHistoryLoadError(message));
    }
  }, [dispatch, migrateFromLocalStorage]);

  const scheduleReconnect = useCallback(() => {
    if (reconnectTimeoutRef.current) return;
    reconnectTimeoutRef.current = setTimeout(() => {
      reconnectTimeoutRef.current = null;
      connectRef.current();
    }, RECONNECT_DELAY_MS);
  }, []);

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

    if (port <= 0 || port > 65535) {
      scheduleReconnect();
      return;
    }

    const onEvent = (envelope: SidebarEventEnvelope) => {
      if (envelope.category === "snapshot") {
        processSnapshot(
          envelope as SidebarEventEnvelope & { category: "snapshot" },
        );
      } else if (envelope.category === "trajectory") {
        processTrajectoryEvent(
          envelope as SidebarEventEnvelope & { category: "trajectory" },
        );
      } else {
        processTaskEvent(
          envelope as SidebarEventEnvelope & { category: "task" },
        );
      }
    };

    const onError = (error: Error) => {
      if (!initialLoadDoneRef.current) {
        dispatch(setHistoryLoadError(error.message));
      }
      scheduleReconnect();
    };

    const onDisconnected = () => {
      scheduleReconnect();
    };

    disconnectRef.current = subscribeToSidebarEvents(port, apiKey, {
      onEvent,
      onError,
      onDisconnected,
    });
  }, [
    dispatch,
    config.lspPort,
    config.apiKey,
    processSnapshot,
    processTrajectoryEvent,
    processTaskEvent,
    scheduleReconnect,
  ]);

  connectRef.current = connect;

  useEffect(() => {
    void loadInitialHistory();
    connect();
    return () => {
      if (disconnectRef.current) {
        disconnectRef.current();
      }
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
    };
  }, [connect, loadInitialHistory]);
}
