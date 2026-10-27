import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "react-redux";
import type { AppStore } from "../app/store";
import type { InternalLinkOrigin } from "../contexts/InternalLinkContext";
import { parseRefactLink } from "../contexts/internalLinkUtils";
import {
  createChatWithId,
  selectBackgroundAgentsByThread,
  selectRuntimeById,
  selectThreadById,
  switchToThread,
} from "../features/Chat/Thread";
import { popBackTo, push } from "../features/Pages/pagesSlice";
import {
  openTask,
  selectTaskActiveChat,
  setTaskActiveChat,
} from "../features/Tasks/tasksSlice";
import { resolveChatLink } from "../features/Tasks/internalLinkResolver";
import { backgroundAgentsApi } from "../services/refact/backgroundAgents";
import { getEngineEndpointIdentity } from "../services/refact/apiUrl";
import {
  tasksApi,
  taskErrorMessage,
  type TaskMeta,
} from "../services/refact/tasks";
import {
  worktreesApi,
  type WorktreeRecordView,
} from "../services/refact/worktrees";
import { trajectoriesApi } from "../services/refact/trajectories";
import type { TrajectoryData } from "../services/refact/trajectories";
import type { ChatThread } from "../features/Chat/Thread/types";
import {
  getProjectStorageNamespace,
  saveTaskWorkspaceTab,
} from "../utils/chatUiPersistence";

const useAppStore = useStore.withTypes<AppStore>();
type NavigationOptions = {
  scope?: string;
  onChatOpened?: (chatId: string) => void;
  onCardOpened?: (taskId: string, cardId: string) => void;
  onWorktreeOpened?: (record: WorktreeRecordView) => void;
};

type NavigationTrajectory = TrajectoryData &
  Pick<ChatThread, "parent_id" | "link_type" | "root_chat_id" | "task_meta">;

export function useInternalNavigation({
  scope = "",
  onChatOpened,
  onCardOpened,
  onWorktreeOpened,
}: NavigationOptions = {}) {
  const store = useAppStore();
  const mounted = useRef(false);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const navigate = useCallback(
    async (url: string, origin: InternalLinkOrigin = {}): Promise<boolean> => {
      const target = parseRefactLink(url);
      if (!target) {
        setError("Unsupported or invalid internal link.");
        return false;
      }
      if (target.type === "worktree" && !onWorktreeOpened) {
        setError(
          "Worktree links cannot be opened here. Use the authorized worktree inspector on the task card or chat.",
        );
        return false;
      }
      if (inFlight.current) return false;
      inFlight.current = true;
      setPending(true);
      setError(null);
      const initial = store.getState();
      const endpoint = getEngineEndpointIdentity(initial.config);
      const apiKey = initial.config.apiKey;
      const project = getProjectStorageNamespace();
      const projectInfo = JSON.stringify(initial.current_project);
      const sourcePage = initial.pages.at(-1);
      const sourceThreadId = initial.chat.current_thread_id;
      const sourceTab = initial.workspace.activeTabId;
      const sourceScope = currentScope.current;
      const sourceTaskChat = origin.taskId
        ? selectTaskActiveChat(initial, origin.taskId)?.chatId
        : undefined;
      let valid = true;
      const checkSource = () => {
        const state = store.getState();
        valid =
          valid &&
          mounted.current &&
          currentScope.current === sourceScope &&
          getEngineEndpointIdentity(state.config) === endpoint &&
          state.config.apiKey === apiKey &&
          getProjectStorageNamespace() === project &&
          JSON.stringify(state.current_project) === projectInfo &&
          state.pages.at(-1) === sourcePage &&
          state.chat.current_thread_id === sourceThreadId &&
          state.workspace.activeTabId === sourceTab &&
          (!origin.taskId ||
            selectTaskActiveChat(state, origin.taskId)?.chatId ===
              sourceTaskChat);
        return valid;
      };
      const unsubscribe = store.subscribe(checkSource);
      const queryOptions = { forceRefetch: true, subscribe: false };
      try {
        if (target.type === "worktree") {
          const inventory = await store
            .dispatch(
              worktreesApi.endpoints.listWorktrees.initiate(
                undefined,
                queryOptions,
              ),
            )
            .unwrap();
          if (!checkSource()) return false;
          const record = inventory.worktrees.find(
            (candidate) => candidate.meta.id === target.id,
          );
          if (
            !record ||
            (target.chatId &&
              !(record.referencing_chat_ids ?? []).includes(target.chatId))
          )
            throw new Error(
              "Worktree is not visible in this project for the linked chat.",
            );
          unsubscribe();
          onWorktreeOpened?.(record);
          return true;
        }
        if (target.type === "task") {
          const task = await store
            .dispatch(
              tasksApi.endpoints.getTask.initiate(target.id, queryOptions),
            )
            .unwrap();
          if (!checkSource()) return false;
          if (task.id !== target.id)
            throw new Error(
              "Task response did not match the requested target.",
            );
          if (target.cardId) {
            const board = await store
              .dispatch(
                tasksApi.endpoints.getBoard.initiate(target.id, queryOptions),
              )
              .unwrap();
            if (!checkSource()) return false;
            if (!board.cards.some((card) => card.id === target.cardId))
              throw new Error(`Card not found on this task: ${target.cardId}`);
          }
          unsubscribe();
          store.dispatch(openTask({ id: task.id, name: task.name }));
          if (target.cardId && onCardOpened && origin.taskId === task.id) {
            onCardOpened(task.id, target.cardId);
          } else {
            store.dispatch(popBackTo({ name: "history" }));
            store.dispatch(
              push({
                name: "task workspace",
                taskId: task.id,
                cardId: target.cardId,
              }),
            );
          }
          return true;
        }

        const data: NavigationTrajectory = await store
          .dispatch(
            trajectoriesApi.endpoints.getTrajectory.initiate(
              target.id,
              queryOptions,
            ),
          )
          .unwrap();
        if (!checkSource()) return false;
        if (data.id !== target.id || !Array.isArray(data.messages))
          throw new Error("Chat response did not match the requested target.");
        const source = origin.chatId
          ? selectThreadById(initial, origin.chatId)
          : undefined;
        const agents = origin.chatId
          ? [
              ...Object.values(
                selectBackgroundAgentsByThread(initial, origin.chatId),
              ),
              ...(backgroundAgentsApi.endpoints.getBackgroundAgents.select(
                origin.chatId,
              )(initial).data ?? []),
            ]
          : [];
        const agent = agents.find(
          (candidate) => candidate.child_chat_id === target.id,
        );
        let taskMeta = data.task_meta;
        let activeChat:
          | { type: "planner"; chatId: string }
          | { type: "agent"; chatId: string; cardId: string }
          | undefined;
        let validatedTask: TaskMeta | undefined;
        const taskId = taskMeta?.task_id ?? origin.taskId;
        if (taskId) {
          if (!parseRefactLink(`refact://task/${taskId}`))
            throw new Error("Invalid task reference in chat response.");
          validatedTask = await store
            .dispatch(tasksApi.endpoints.getTask.initiate(taskId, queryOptions))
            .unwrap();
          if (!checkSource()) return false;
          if (validatedTask.id !== taskId)
            throw new Error("Task response did not match the chat.");
          const board = await store
            .dispatch(
              tasksApi.endpoints.getBoard.initiate(taskId, queryOptions),
            )
            .unwrap();
          if (!checkSource()) return false;
          const planners = await store
            .dispatch(
              tasksApi.endpoints.listTaskTrajectories.initiate(
                { taskId, role: "planner" },
                queryOptions,
              ),
            )
            .unwrap();
          if (!checkSource()) return false;
          const resolved = resolveChatLink(target.id, planners, board);
          if (
            resolved.kind === "agent" &&
            (!taskMeta ||
              (taskMeta.role === "agents" &&
                taskMeta.card_id === resolved.cardId))
          ) {
            activeChat = {
              type: "agent",
              chatId: target.id,
              cardId: resolved.cardId,
            };
            taskMeta = {
              ...taskMeta,
              task_id: taskId,
              role: "agents",
              card_id: resolved.cardId,
            };
          } else if (
            resolved.kind === "planner" &&
            (!taskMeta || taskMeta.role === "planner")
          ) {
            activeChat = { type: "planner", chatId: target.id };
            taskMeta = { ...taskMeta, task_id: taskId, role: "planner" };
          } else if (taskMeta)
            throw new Error(
              "This chat is not a current planner or agent for the referenced task card.",
            );
        }
        unsubscribe();
        if (!selectRuntimeById(store.getState(), target.id)) {
          store.dispatch(
            createChatWithId({
              id: data.id,
              title: data.title,
              model: data.model,
              mode: data.mode,
              parentId: data.parent_id ?? agent?.parent_chat_id,
              linkType: data.link_type ?? agent?.kind,
              rootChatId:
                data.root_chat_id ??
                (source && agent?.parent_chat_id === source.id
                  ? source.root_chat_id ?? source.id
                  : undefined),
              isTaskChat: Boolean(taskMeta),
              taskMeta,
              openTab: false,
              activate: false,
            }),
          );
        }
        if (validatedTask && activeChat) {
          store.dispatch(
            openTask({ id: validatedTask.id, name: validatedTask.name }),
          );
          store.dispatch(
            setTaskActiveChat({ taskId: validatedTask.id, activeChat }),
          );
          if (origin.taskId === validatedTask.id && onChatOpened)
            onChatOpened(target.id);
          else {
            saveTaskWorkspaceTab(validatedTask.id, "chat");
            store.dispatch(popBackTo({ name: "history" }));
            store.dispatch(
              push({ name: "task workspace", taskId: validatedTask.id }),
            );
          }
          store.dispatch(switchToThread({ id: target.id, openTab: false }));
        } else {
          store.dispatch(switchToThread({ id: target.id }));
          store.dispatch(popBackTo({ name: "history" }));
          store.dispatch(push({ name: "chat" }));
        }
        return true;
      } catch (cause) {
        if (checkSource())
          setError(`Could not open link: ${taskErrorMessage(cause)}`);
        return false;
      } finally {
        unsubscribe();
        inFlight.current = false;
        if (mounted.current) setPending(false);
      }
    },
    [onCardOpened, onChatOpened, onWorktreeOpened, store],
  );

  return { navigate, pending, error };
}
