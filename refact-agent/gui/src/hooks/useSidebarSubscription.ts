import { useEffect, useRef, useCallback } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import { usePostMessage } from "./usePostMessage";
import {
  subscribeToSidebarEvents,
  SidebarEventEnvelope,
  type BuddySnapshotPayload,
  type NotificationEvent,
  type SidebarSection,
  type SidebarSectionSnapshot,
  type SidebarSectionUpdate,
  type TaskEvent,
} from "../services/refact/sidebarSubscription";
import type { BuddySSEEvent } from "../features/Buddy/types";
import type {
  TrajectoryMeta,
  TrajectoryEvent,
} from "../services/refact/trajectories";
import {
  hydrateHistoryFromMeta,
  replaceSnapshotHistory,
  deleteChatById,
  updateChatMetaById,
  setHistoryLoading,
  setHistoryLoadError,
} from "../features/History/historySlice";
import type { ChatHistoryItem } from "../features/History/historySlice";
import {
  updateOpenThread,
  closeThread,
  updateChatRuntimeFromSessionState,
} from "../features/Chat/Thread";
import { setCurrentProjectInfo } from "../features/Chat/currentProject";
import { tasksApi, type TaskMeta } from "../services/refact/tasks";
import {
  setBuddySnapshot,
  setBuddyUnavailable,
  updateBuddyState,
  addBuddyActivity,
  addBuddySuggestion,
  dismissBuddySuggestion,
  updateBuddySettings,
  addBuddyDiagnostic,
  enqueueRuntimeEvent,
  setActiveSpeech,
  addOpportunity,
  resolveOpportunity,
  setPulse,
  addDraft,
  consumeDraft,
  removeDraft,
} from "../features/Buddy/buddySlice";
import { executeBuddyNavigation } from "../features/Buddy/executeBuddyAction";

import {
  trajectoriesApi,
  chatThreadToTrajectoryData,
} from "../services/refact/trajectories";
import { useAppSelector } from "./useAppSelector";
import { ideAskQuestions, ideTaskDone } from "./useEventBusForIDE";
import {
  resetSidebarState,
  sidebarSectionSnapshotReceived,
  sidebarSubscriptionStarted,
  sidebarWorkspaceChanged,
} from "../features/Sidebar/sidebarSlice";

const RECONNECT_DELAY_MS = 500;
const MIGRATION_KEY = "refact-trajectories-migrated";

function getWorkspaceDisplayName(root: string): string {
  const trimmed = root.trim();
  if (!trimmed) return "";
  const normalized = trimmed.replace(/\\/g, "/").replace(/\/+$/, "");
  return normalized.split("/").pop() ?? normalized;
}

function normalizeWorkspaceRoot(root: string): string {
  return root.trim().replace(/\\/g, "/").replace(/\/+$/, "");
}

function canonicalWorkspaceRoots(roots: string[]): string[] {
  return Array.from(new Set(roots.map(normalizeWorkspaceRoot))).sort();
}

function workspaceRootsEqual(
  current: string[] | undefined,
  next: string[],
): boolean {
  if (!current) return false;
  const currentCanonical = canonicalWorkspaceRoots(current);
  const nextCanonical = canonicalWorkspaceRoots(next);
  if (currentCanonical.length !== nextCanonical.length) return false;

  return currentCanonical.every((root, index) => root === nextCanonical[index]);
}

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

function trajectoryItemsFromMeta(
  trajectories: TrajectoryMeta[],
): TrajectoryMeta[] {
  return trajectories.map((t) => ({
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
    worktree: t.worktree,
    total_lines_added: t.total_lines_added,
    total_lines_removed: t.total_lines_removed,
    tasks_total: t.tasks_total,
    tasks_done: t.tasks_done,
    tasks_failed: t.tasks_failed,
    task_id: t.task_id,
    task_role: t.task_role,
    agent_id: t.agent_id,
    card_id: t.card_id,
  }));
}

function hasSnapshotKey<K extends string>(
  snapshot: SidebarSectionSnapshot,
  key: K,
): snapshot is Extract<SidebarSectionSnapshot, Record<K, unknown>> {
  return key in snapshot;
}

function isTrajectoryUpdate(
  update: SidebarSectionUpdate,
): update is TrajectoryEvent {
  return "type" in update && "id" in update;
}

function isTaskUpdate(update: SidebarSectionUpdate): update is TaskEvent {
  return "type" in update && !("id" in update);
}

function isBuddyUpdate(update: SidebarSectionUpdate): update is BuddySSEEvent {
  return "event_type" in update;
}

type SidebarSnapshotEvent = Extract<
  SidebarEventEnvelope["event"],
  { type: "section_snapshot" }
>;
type SidebarSectionSnapshotStatus = SidebarSnapshotEvent["status"];
type SidebarRuntimeSectionStatus = SidebarSectionSnapshotStatus | "loading";
type NonWorkspaceSidebarSection = Exclude<SidebarSection, "workspace">;
type BufferedSectionSnapshots = Partial<
  Record<NonWorkspaceSidebarSection, SidebarSnapshotEvent>
>;

function initialSectionStatuses(): Record<
  SidebarSection,
  SidebarRuntimeSectionStatus
> {
  return {
    workspace: "loading",
    chats: "loading",
    tasks: "loading",
    buddy: "loading",
  };
}

function sectionStatusesSettled(
  statuses: Record<SidebarSection, SidebarRuntimeSectionStatus>,
): boolean {
  return Object.values(statuses).every((status) => status !== "loading");
}

function isNonWorkspaceSection(
  section: SidebarSection,
): section is NonWorkspaceSidebarSection {
  return section !== "workspace";
}

export function useSidebarSubscription() {
  const dispatch = useAppDispatch();
  const config = useConfig();
  const postMessage = usePostMessage();
  const historyChats = useAppSelector((state) => state.history.chats);
  const historyRef = useRef(historyChats);
  historyRef.current = historyChats;
  const serverWorkspaceRootsRef = useRef<string[] | undefined>(undefined);
  const disconnectRef = useRef<(() => void) | null>(null);
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const tasksSnapshotRef = useRef<TaskMeta[] | null>(null);
  const generationRef = useRef(0);
  const sectionStatusesRef = useRef(initialSectionStatuses());
  const postSettledSnapshotsRef = useRef<BufferedSectionSnapshots>({});
  // eslint-disable-next-line @typescript-eslint/no-empty-function
  const connectRef = useRef<() => void>(() => {});

  const processTrajectoryEvent = useCallback(
    (event: TrajectoryEvent) => {
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
        event.worktree !== undefined ||
        event.total_lines_added !== undefined ||
        event.total_lines_removed !== undefined ||
        event.tasks_total !== undefined ||
        event.tasks_done !== undefined ||
        event.tasks_failed !== undefined ||
        event.task_id !== undefined ||
        event.task_role !== undefined ||
        event.agent_id !== undefined ||
        event.card_id !== undefined;

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
        if (event.worktree !== undefined) metaPatch.worktree = event.worktree;
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
        if (event.task_id !== undefined) metaPatch.task_id = event.task_id;
        if (event.task_role !== undefined)
          metaPatch.task_role = event.task_role;
        if (event.agent_id !== undefined) metaPatch.agent_id = event.agent_id;
        if (event.card_id !== undefined) metaPatch.card_id = event.card_id;
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
              worktree: event.worktree,
              total_lines_added: event.total_lines_added ?? 0,
              total_lines_removed: event.total_lines_removed ?? 0,
              tasks_total: 0,
              tasks_done: 0,
              tasks_failed: 0,
              task_id: event.task_id,
              task_role: event.task_role,
              agent_id: event.agent_id,
              card_id: event.card_id,
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

  const updateListTasksCache = useCallback(
    (updater: (draft: TaskMeta[]) => void) => {
      const next = [...(tasksSnapshotRef.current ?? [])];
      updater(next);
      tasksSnapshotRef.current = next;
      void dispatch(
        tasksApi.util.upsertQueryData("listTasks", undefined, next),
      );
    },
    [dispatch],
  );

  const processTaskEvent = useCallback(
    (event: Extract<SidebarSectionUpdate, { type: string }>) => {
      switch (event.type) {
        case "snapshot":
          tasksSnapshotRef.current = event.tasks;
          void dispatch(
            tasksApi.util.upsertQueryData("listTasks", undefined, event.tasks),
          );
          break;

        case "task_created":
          updateListTasksCache((draft) => {
            const exists = draft.some((t) => t.id === event.task_id);
            if (!exists) {
              draft.unshift(event.meta);
            }
          });
          break;

        case "task_updated":
          updateListTasksCache((draft) => {
            const index = draft.findIndex((t) => t.id === event.task_id);
            if (index >= 0) {
              const existing = draft[index];
              draft[index] = {
                ...event.meta,
                planner_session_state:
                  event.meta.planner_session_state ??
                  existing.planner_session_state,
              };
            } else {
              draft.unshift(event.meta);
            }
            draft.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
          });
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
          updateListTasksCache((draft) => {
            const index = draft.findIndex((t) => t.id === event.task_id);
            if (index >= 0) {
              draft.splice(index, 1);
            }
          });
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
    [dispatch, updateListTasksCache],
  );

  const processWorkspaceSnapshot = useCallback(
    (workspaceRoots: string[]) => {
      dispatch(
        setCurrentProjectInfo({
          name: getWorkspaceDisplayName(workspaceRoots[0] ?? ""),
          workspaceRoots,
        }),
      );
    },
    [dispatch],
  );

  const processTrajectoriesSnapshot = useCallback(
    (trajectories: TrajectoryMeta[], error?: string) => {
      if (trajectories.length > 0 || !error) {
        dispatch(replaceSnapshotHistory(trajectoryItemsFromMeta(trajectories)));
      }
      dispatch(setHistoryLoadError(error ?? null));
      dispatch(setHistoryLoading(false));
    },
    [dispatch],
  );

  const processTasksSnapshot = useCallback(
    (tasks: TaskMeta[]) => {
      tasksSnapshotRef.current = tasks;
      void dispatch(
        tasksApi.util.upsertQueryData("listTasks", undefined, tasks),
      );
    },
    [dispatch],
  );

  const processBuddySnapshot = useCallback(
    (buddy: BuddySnapshotPayload | undefined) => {
      if (!buddy) {
        dispatch(setBuddyUnavailable());
      } else {
        dispatch(setBuddySnapshot(buddy));
      }
    },
    [dispatch],
  );

  const processSectionSnapshotData = useCallback(
    (
      section: SidebarSection,
      snapshot: SidebarSectionSnapshot,
      status: SidebarSnapshotEvent["status"],
      error?: string,
    ) => {
      if (
        section === "workspace" &&
        status === "ready" &&
        hasSnapshotKey(snapshot, "workspace_roots")
      ) {
        const workspaceRoots = canonicalWorkspaceRoots(
          snapshot.workspace_roots,
        );
        processWorkspaceSnapshot(workspaceRoots);
        serverWorkspaceRootsRef.current = workspaceRoots;
      } else if (
        section === "chats" &&
        hasSnapshotKey(snapshot, "trajectories")
      ) {
        processTrajectoriesSnapshot(
          snapshot.trajectories,
          status === "error" ? error ?? "Failed to load chats" : undefined,
        );
      } else if (
        section === "tasks" &&
        status === "ready" &&
        hasSnapshotKey(snapshot, "tasks")
      ) {
        processTasksSnapshot(snapshot.tasks);
      } else if (section === "buddy" && hasSnapshotKey(snapshot, "buddy")) {
        processBuddySnapshot(snapshot.buddy);
      }
    },
    [
      processBuddySnapshot,
      processTasksSnapshot,
      processTrajectoriesSnapshot,
      processWorkspaceSnapshot,
    ],
  );

  const markSectionSnapshotReceived = useCallback(
    (event: SidebarSnapshotEvent) => {
      dispatch(
        sidebarSectionSnapshotReceived({
          section: event.section,
          status: event.status,
          error: event.status === "error" ? event.error : null,
        }),
      );
      sectionStatusesRef.current = {
        ...sectionStatusesRef.current,
        [event.section]: event.status,
      };
    },
    [dispatch],
  );

  const replayBufferedSnapshots = useCallback(
    (snapshots: BufferedSectionSnapshots) => {
      for (const section of ["chats", "tasks", "buddy"] as const) {
        const event = snapshots[section];
        if (!event) continue;
        processSectionSnapshotData(
          event.section,
          event.snapshot,
          event.status,
          event.error,
        );
        markSectionSnapshotReceived(event);
      }
    },
    [markSectionSnapshotReceived, processSectionSnapshotData],
  );

  const processSectionSnapshot = useCallback(
    (
      event: Extract<
        SidebarEventEnvelope["event"],
        { type: "section_snapshot" }
      >,
      subscriptionId: string,
    ) => {
      const { section, snapshot, status, error } = event;

      if (
        isNonWorkspaceSection(section) &&
        sectionStatusesSettled(sectionStatusesRef.current)
      ) {
        postSettledSnapshotsRef.current = {
          ...postSettledSnapshotsRef.current,
          [section]: event,
        };
      }

      if (
        section === "workspace" &&
        hasSnapshotKey(snapshot, "workspace_roots")
      ) {
        const workspaceRoots = canonicalWorkspaceRoots(
          snapshot.workspace_roots,
        );
        const workspaceChanged =
          status === "ready" &&
          serverWorkspaceRootsRef.current !== undefined &&
          !workspaceRootsEqual(serverWorkspaceRootsRef.current, workspaceRoots);
        const snapshotsToReplay = workspaceChanged
          ? postSettledSnapshotsRef.current
          : undefined;
        if (status === "ready") {
          postSettledSnapshotsRef.current = {};
        }
        if (workspaceChanged) {
          dispatch(sidebarWorkspaceChanged({ subscriptionId }));
          sectionStatusesRef.current = initialSectionStatuses();
          tasksSnapshotRef.current = null;
          dispatch(replaceSnapshotHistory([]));
          void dispatch(
            tasksApi.util.upsertQueryData("listTasks", undefined, []),
          );
          dispatch(setHistoryLoading(true));
        }
        if (status === "ready") {
          processWorkspaceSnapshot(workspaceRoots);
          serverWorkspaceRootsRef.current = workspaceRoots;
        }
        markSectionSnapshotReceived(event);
        if (workspaceChanged && snapshotsToReplay) {
          replayBufferedSnapshots(snapshotsToReplay);
        }
        return;
      }

      processSectionSnapshotData(section, snapshot, status, error);
      markSectionSnapshotReceived(event);
    },
    [
      dispatch,
      markSectionSnapshotReceived,
      processSectionSnapshotData,
      processWorkspaceSnapshot,
      replayBufferedSnapshots,
    ],
  );

  const processNotification = useCallback(
    (event: NotificationEvent) => {
      if (event.type === "task_done") {
        postMessage(
          ideTaskDone({
            chatId: event.chat_id,
            toolCallId: event.tool_call_id,
            summary: event.summary,
            knowledgePath: event.knowledge_path,
          }),
        );
        return;
      }

      postMessage(
        ideAskQuestions({
          chatId: event.chat_id,
          toolCallId: event.tool_call_id,
          questions: event.questions,
        }),
      );
    },
    [postMessage],
  );

  const processBuddyEvent = useCallback(
    (event: Extract<SidebarSectionUpdate, { event_type: string }>) => {
      switch (event.event_type) {
        case "StateUpdated":
          dispatch(updateBuddyState(event.state));
          break;
        case "ActivityAdded":
          dispatch(addBuddyActivity(event.activity));
          break;
        case "SuggestionAdded":
          dispatch(addBuddySuggestion(event.suggestion));
          break;
        case "SuggestionDismissed":
          dispatch(dismissBuddySuggestion(event.suggestion_id));
          break;
        case "SettingsChanged":
          dispatch(updateBuddySettings(event.settings));
          break;
        case "DiagnosticAdded":
          dispatch(addBuddyDiagnostic(event.diagnostic));
          break;
        case "RuntimeEvent":
          dispatch(enqueueRuntimeEvent(event.event));
          break;
        case "SpeechUpdated":
          dispatch(setActiveSpeech(event.speech));
          break;
        case "NavigationRequest":
          executeBuddyNavigation(event.page, dispatch);
          break;
        case "OpportunityProduced":
          dispatch(addOpportunity(event.opportunity));
          break;
        case "OpportunityResolved":
          dispatch(
            resolveOpportunity({
              id: event.opportunity_id,
              status: event.status,
            }),
          );
          break;
        case "PulseUpdated":
          dispatch(setPulse(event.pulse));
          break;
        case "DraftCreated":
          dispatch(addDraft(event.draft));
          break;
        case "DraftConsumed":
          dispatch(consumeDraft(event.draft_id));
          break;
        case "DraftRemoved":
          dispatch(removeDraft(event.draft_id));
          break;
      }
    },
    [dispatch],
  );

  const processSectionUpdate = useCallback(
    (section: SidebarSection, update: SidebarSectionUpdate) => {
      if (section === "chats" && isTrajectoryUpdate(update)) {
        processTrajectoryEvent(update);
      } else if (section === "tasks" && isTaskUpdate(update)) {
        processTaskEvent(update);
      } else if (section === "buddy" && isBuddyUpdate(update)) {
        processBuddyEvent(update);
      }
    },
    [processBuddyEvent, processTaskEvent, processTrajectoryEvent],
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

  const prepareInitialHistory = useCallback(
    async (generation: number) => {
      if (generation !== generationRef.current) return;
      dispatch(setHistoryLoading(true));
      try {
        await migrateFromLocalStorage();
        if (generation !== generationRef.current) return;
      } catch (err) {
        if (generation !== generationRef.current) return;
        const message =
          err instanceof Error
            ? err.message
            : "Failed to migrate local history";
        dispatch(setHistoryLoadError(message));
      }
    },
    [dispatch, migrateFromLocalStorage],
  );

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

    const generation = ++generationRef.current;
    dispatch(resetSidebarState({ lspPort: port }));
    serverWorkspaceRootsRef.current = undefined;
    tasksSnapshotRef.current = null;
    sectionStatusesRef.current = initialSectionStatuses();
    postSettledSnapshotsRef.current = {};
    void prepareInitialHistory(generation);

    const onEvent = (envelope: SidebarEventEnvelope) => {
      if (generation !== generationRef.current) return;

      if (envelope.seq === 0) {
        dispatch(
          sidebarSubscriptionStarted({
            subscriptionId: envelope.subscription_id,
            lspPort: port,
          }),
        );
      }

      if (envelope.event.type === "section_snapshot") {
        processSectionSnapshot(envelope.event, envelope.subscription_id);
      } else if (envelope.event.type === "section_update") {
        processSectionUpdate(envelope.event.section, envelope.event.update);
      } else {
        processNotification(envelope.event.notification);
      }
    };

    const onError = (error: Error) => {
      if (generation !== generationRef.current) return;
      dispatch(
        sidebarSectionSnapshotReceived({
          section: "chats",
          status: "error",
          error: error.message,
        }),
      );
      dispatch(setHistoryLoadError(error.message));
      dispatch(setHistoryLoading(false));
      scheduleReconnect();
    };

    const onDisconnected = () => {
      if (generation !== generationRef.current) return;
      scheduleReconnect();
    };

    disconnectRef.current = subscribeToSidebarEvents(port, apiKey, {
      onEvent,
      onError,
      onDisconnected,
    });
  }, [
    config.lspPort,
    config.apiKey,
    dispatch,
    prepareInitialHistory,
    processNotification,
    processSectionSnapshot,
    processSectionUpdate,
    scheduleReconnect,
  ]);

  connectRef.current = connect;

  useEffect(() => {
    connect();
    return () => {
      generationRef.current += 1;
      if (disconnectRef.current) {
        disconnectRef.current();
      }
      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current);
      }
    };
  }, [connect]);
}
