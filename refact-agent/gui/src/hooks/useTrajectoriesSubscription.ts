import { useEffect, useRef, useCallback } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import {
  trajectoriesApi,
  TrajectoryEvent,
  chatThreadToTrajectoryData,
} from "../services/refact/trajectories";
import {
  hydrateHistoryFromMeta,
  deleteChatById,
  updateChatMetaById,
  setHistoryLoading,
  setHistoryLoadError,
  setPagination,
} from "../features/History/historySlice";
import type { ChatHistoryItem } from "../features/History/historySlice";
import { updateOpenThread, closeThread } from "../features/Chat/Thread";
import { useAppSelector } from "./useAppSelector";

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
    // ignore
  }
}

function isMigrationDone(): boolean {
  return localStorage.getItem(MIGRATION_KEY) === "true";
}

function markMigrationDone() {
  localStorage.setItem(MIGRATION_KEY, "true");
}

export function useTrajectoriesSubscription(): {
  retryInitialLoad: () => void;
} {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const historyChats = useAppSelector((state) => state.history.chats);
  const historyRef = useRef(historyChats);
  historyRef.current = historyChats;
  const abortControllerRef = useRef<AbortController | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const processEvent = useCallback(
    (data: TrajectoryEvent) => {
      if (data.type === "deleted") {
        dispatch(deleteChatById(data.id));
        dispatch(closeThread({ id: data.id, force: true }));
      } else {
        const existsInHistory = data.id in historyRef.current;
        const hasMetaUpdate =
          data.title !== undefined ||
          data.updated_at !== undefined ||
          data.session_state !== undefined ||
          data.message_count !== undefined ||
          data.parent_id !== undefined ||
          data.link_type !== undefined ||
          data.root_chat_id !== undefined;
        if (existsInHistory && hasMetaUpdate) {
          dispatch(
            updateChatMetaById({
              id: data.id,
              title: data.title,
              updatedAt: data.updated_at,
              session_state: data.session_state,
              message_count: data.message_count,
              parent_id: data.parent_id,
              link_type: data.link_type,
              root_chat_id: data.root_chat_id,
            }),
          );
          if (data.title) {
            dispatch(
              updateOpenThread({
                id: data.id,
                thread: { title: data.title, isTitleGenerated: true },
              }),
            );
          }
        } else if (!existsInHistory && data.title && data.updated_at) {
          dispatch(
            hydrateHistoryFromMeta([
              {
                id: data.id,
                title: data.title,
                created_at: data.updated_at,
                updated_at: data.updated_at,
                model: data.model ?? "",
                mode: data.mode ?? "AGENT",
                message_count: data.message_count ?? 0,
                session_state: data.session_state,
                parent_id: data.parent_id,
                link_type: data.link_type,
                root_chat_id: data.root_chat_id,
              },
            ]),
          );
          dispatch(
            updateOpenThread({
              id: data.id,
              thread: { title: data.title, isTitleGenerated: true },
            }),
          );
        }
      }
    },
    [dispatch],
  );

  const connect = useCallback(() => {
    const port = config.lspPort;
    const apiKey = config.apiKey;
    const url = `http://127.0.0.1:${port}/v1/trajectories/subscribe`;

    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
    }

    const abortController = new AbortController();
    abortControllerRef.current = abortController;

    const headers: Record<string, string> = {};
    if (apiKey) {
      headers.Authorization = `Bearer ${apiKey}`;
    }

    void fetch(url, {
      method: "GET",
      headers,
      signal: abortController.signal,
    })
      .then(async (response) => {
        if (!response.ok || !response.body) {
          throw new Error(`SSE connection failed: ${response.status}`);
        }

        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";

        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });
          buffer = buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

          const blocks = buffer.split("\n\n");
          buffer = blocks.pop() ?? "";

          for (const block of blocks) {
            if (!block.trim()) continue;

            const dataLines: string[] = [];
            for (const rawLine of block.split("\n")) {
              if (rawLine.startsWith(":")) continue;
              if (!rawLine.startsWith("data:")) continue;
              dataLines.push(rawLine.slice(5).replace(/^\s*/, ""));
            }

            if (dataLines.length === 0) continue;

            const dataStr = dataLines.join("\n");
            try {
              const data = JSON.parse(dataStr) as TrajectoryEvent;
              processEvent(data);
            } catch {
              // skip malformed events
            }
          }
        }

        if (reconnectTimeoutRef.current) {
          clearTimeout(reconnectTimeoutRef.current);
        }
        reconnectTimeoutRef.current = setTimeout(connect, 5000);
      })
      .catch((err: unknown) => {
        const error = err as Error;
        if (error.name === "AbortError") return;

        if (reconnectTimeoutRef.current) {
          clearTimeout(reconnectTimeoutRef.current);
        }
        reconnectTimeoutRef.current = setTimeout(connect, 5000);
      });
  }, [config.lspPort, config.apiKey, processEvent]);

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
        // Failed to migrate this chat, continue with others
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
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Failed to load history";
      dispatch(setHistoryLoadError(message));
    }
  }, [dispatch, migrateFromLocalStorage]);

  const retryInitialLoad = useCallback(() => {
    void loadInitialHistory();
  }, [loadInitialHistory]);

  useEffect(() => {
    void loadInitialHistory();
    connect();

    return () => {
      if (abortControllerRef.current) {
        abortControllerRef.current.abort();
      }
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
    };
  }, [connect, loadInitialHistory]);

  return { retryInitialLoad };
}
