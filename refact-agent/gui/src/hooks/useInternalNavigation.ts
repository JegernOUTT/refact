import { useCallback, useEffect, useRef, useState } from "react";
import { useStore } from "react-redux";
import type { AppStore } from "../app/store";
import type { InternalLinkOrigin } from "../contexts/InternalLinkContext";
import { parseRefactLink } from "../contexts/internalLinkUtils";
import { selectAgentRosterSource } from "../features/AgentsPanel/agentsPanelSlice";
import {
  buildBackgroundAgentsTree,
  createChatWithId,
  flattenBackgroundAgentTree,
  reconcileBackgroundAgents,
  selectBackgroundAgentPool,
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
  type TaskBoard,
} from "../services/refact/tasks";
import {
  worktreesApi,
  type WorktreeRecordView,
} from "../services/refact/worktrees";
import { trajectoriesApi } from "../services/refact/trajectories";
import {
  getProjectStorageNamespace,
  saveTaskWorkspaceTab,
} from "../utils/chatUiPersistence";

const useAppStore = useStore.withTypes<AppStore>();
export const INTERNAL_NAVIGATION_TIMEOUT_MS = 15_000;
type LocalTaskChatContext = {
  task: TaskMeta;
  board: TaskBoard;
  plannerChats: { id: string }[];
  pendingPlannerIds?: ReadonlySet<string>;
  deletedPlannerIds?: ReadonlySet<string>;
};
type NavigationOptions = {
  scope?: string;
  localTaskChatContext?: LocalTaskChatContext;
  onChatOpened?: (chatId: string) => void;
  onCardOpened?: (taskId: string, cardId: string) => void;
  onWorktreeOpened?: (record: WorktreeRecordView) => void;
};

export function useInternalNavigation({
  scope = "",
  localTaskChatContext,
  onChatOpened,
  onCardOpened,
  onWorktreeOpened,
}: NavigationOptions = {}) {
  const store = useAppStore();
  const mounted = useRef(false);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const localContext = useRef(localTaskChatContext);
  localContext.current = localTaskChatContext;
  const inFlight = useRef(false);
  const cancellation = useRef<AbortController | null>(null);
  const [pending, setPending] = useState(false);
  const [localPending, setLocalPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      cancellation.current?.abort();
    };
  }, []);
  useEffect(() => () => cancellation.current?.abort(), [scope]);

  const getLocalContext = useCallback(() => {
    const context = localContext.current;
    if (!context) return undefined;
    const state = store.getState();
    const task = tasksApi.endpoints.getTask.select(context.task.id)(state);
    const board = tasksApi.endpoints.getBoard.select(context.task.id)(state);
    const planners = tasksApi.endpoints.listTaskTrajectories.select({
      taskId: context.task.id,
      role: "planner",
    })(state);
    if (task.isError || board.isError || planners.isError) return undefined;
    const saved = planners.data ?? context.plannerChats;
    const plannerChats = saved.filter(
      (planner) => !context.deletedPlannerIds?.has(planner.id),
    );
    for (const planner of context.plannerChats) {
      if (
        context.pendingPlannerIds?.has(planner.id) &&
        !context.deletedPlannerIds?.has(planner.id) &&
        !plannerChats.some((savedPlanner) => savedPlanner.id === planner.id)
      ) {
        plannerChats.push(planner);
      }
    }
    return {
      ...context,
      task: task.data ?? context.task,
      board: board.data ?? context.board,
      plannerChats,
    };
  }, [store]);

  const knownAgent = useCallback(
    (chatId: string, sourceId: string) => {
      const state = store.getState();
      return flattenBackgroundAgentTree(
        buildBackgroundAgentsTree(
          reconcileBackgroundAgents(
            selectBackgroundAgentPool(state),
            backgroundAgentsApi.endpoints.getBackgroundAgents.select({
              chatId: sourceId,
              source: selectAgentRosterSource(state),
            })(state).data,
          ),
          sourceId,
        ),
      ).find((agent) => agent.child_chat_id === chatId);
    },
    [store],
  );

  const runNavigation = useCallback(
    async (
      url: string,
      origin: InternalLinkOrigin = {},
      local?: { context: LocalTaskChatContext; owner: boolean },
      directAgent?: NonNullable<ReturnType<typeof knownAgent>>,
    ): Promise<boolean> => {
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
      setLocalPending(Boolean(local ?? directAgent));
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
      const agentValidity = { current: true };
      const controller = new AbortController();
      cancellation.current = controller;
      const checkSource = () => {
        const state = store.getState();
        if (directAgent) {
          const latest = origin.chatId && knownAgent(target.id, origin.chatId);
          agentValidity.current =
            agentValidity.current &&
            Boolean(
              latest &&
                latest.agent_id === directAgent.agent_id &&
                latest.parent_chat_id === directAgent.parent_chat_id &&
                latest.child_chat_id === directAgent.child_chat_id &&
                latest.kind === directAgent.kind,
            );
        }
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
        if (!valid) controller.abort();
        return valid;
      };
      const unsubscribe = store.subscribe(checkSource);
      const deadline = { timedOut: false };
      let rejectCancellation: (reason: Error) => void = () => undefined;
      const cancelled = new Promise<never>((_resolve, reject) => {
        rejectCancellation = reject;
      });
      const onAbort = () =>
        rejectCancellation(new Error("Navigation cancelled."));
      controller.signal.addEventListener("abort", onAbort, { once: true });
      const timer = setTimeout(() => {
        deadline.timedOut = true;
        controller.abort();
      }, INTERNAL_NAVIGATION_TIMEOUT_MS);
      const read = async <T>(request: {
        abort: () => void;
        unwrap: () => Promise<T>;
      }): Promise<T> => {
        try {
          return await Promise.race([request.unwrap(), cancelled]);
        } finally {
          if (controller.signal.aborted) request.abort();
        }
      };
      const queryOptions = { forceRefetch: true, subscribe: false };
      try {
        if (target.type === "worktree") {
          const inventory = await read(
            store.dispatch(
              worktreesApi.endpoints.listWorktrees.initiate(
                undefined,
                queryOptions,
              ),
            ),
          );
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
          const task = await read(
            store.dispatch(
              tasksApi.endpoints.getTask.initiate(target.id, queryOptions),
            ),
          );
          if (!checkSource()) return false;
          if (task.id !== target.id)
            throw new Error(
              "Task response did not match the requested target.",
            );
          if (target.cardId) {
            const board = await read(
              store.dispatch(
                tasksApi.endpoints.getBoard.initiate(target.id, queryOptions),
              ),
            );
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

        const data = await read(
          store.dispatch(
            trajectoriesApi.endpoints.getTrajectoryMetadata.initiate(
              target.id,
              queryOptions,
            ),
          ),
        );
        if (!checkSource()) return false;
        if (data.id !== target.id)
          throw new Error("Chat response did not match the requested target.");
        if (
          directAgent &&
          data.parent_id &&
          data.parent_id !== directAgent.parent_chat_id
        )
          throw new Error(
            "This agent is no longer a current child of this chat.",
          );
        const source = origin.chatId
          ? selectThreadById(initial, origin.chatId)
          : undefined;
        const agents = origin.chatId
          ? [
              ...Object.values(
                selectBackgroundAgentsByThread(initial, origin.chatId),
              ),
              ...(backgroundAgentsApi.endpoints.getBackgroundAgents.select({
                chatId: origin.chatId,
                source: selectAgentRosterSource(initial),
              })(initial).data ?? []),
            ]
          : [];
        const agent =
          directAgent ??
          agents.find((candidate) => candidate.child_chat_id === target.id);
        let taskMeta = data.task_meta;
        let activeChat:
          | { type: "planner"; chatId: string }
          | { type: "agent"; chatId: string; cardId: string }
          | undefined;
        let validatedTask: TaskMeta | undefined;
        const taskId = taskMeta?.task_id ?? origin.taskId;
        if (local) {
          const latest = getLocalContext();
          const original = resolveChatLink(
            target.id,
            local.context.plannerChats,
            local.context.board,
          );
          const resolved =
            latest &&
            resolveChatLink(target.id, latest.plannerChats, latest.board);
          if (
            !latest ||
            latest.task.id !== local.context.task.id ||
            !resolved ||
            resolved.kind === "unknown" ||
            resolved.kind !== original.kind ||
            (resolved.kind === "agent" &&
              original.kind === "agent" &&
              resolved.cardId !== original.cardId) ||
            (local.owner && latest.board.owner_chat_id !== target.id) ||
            (taskMeta &&
              (taskMeta.task_id !== latest.task.id ||
                taskMeta.role !==
                  (resolved.kind === "planner" ? "planner" : "agents") ||
                (resolved.kind === "agent" &&
                  taskMeta.card_id !== resolved.cardId)))
          )
            throw new Error(
              "This chat is not a current planner or agent for the referenced task card.",
            );
          validatedTask = latest.task;
          activeChat =
            resolved.kind === "planner"
              ? { type: "planner", chatId: target.id }
              : { type: "agent", chatId: target.id, cardId: resolved.cardId };
          taskMeta = {
            ...taskMeta,
            task_id: latest.task.id,
            role: resolved.kind === "planner" ? "planner" : "agents",
            ...(resolved.kind === "agent" ? { card_id: resolved.cardId } : {}),
          };
        } else if (taskId) {
          if (!parseRefactLink(`refact://task/${taskId}`))
            throw new Error("Invalid task reference in chat response.");
          validatedTask = await read(
            store.dispatch(
              tasksApi.endpoints.getTask.initiate(taskId, queryOptions),
            ),
          );
          if (!checkSource()) return false;
          if (validatedTask.id !== taskId)
            throw new Error("Task response did not match the chat.");
          const board = await read(
            store.dispatch(
              tasksApi.endpoints.getBoard.initiate(taskId, queryOptions),
            ),
          );
          if (!checkSource()) return false;
          const planners = await read(
            store.dispatch(
              tasksApi.endpoints.listTaskTrajectories.initiate(
                { taskId, role: "planner" },
                queryOptions,
              ),
            ),
          );
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
        if (!agentValidity.current)
          throw new Error(
            "This agent is no longer a current child of this chat.",
          );
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
          setError(
            deadline.timedOut
              ? "Could not open link: Navigation timed out. Please try again."
              : `Could not open link: ${taskErrorMessage(cause)}`,
          );
        return false;
      } finally {
        unsubscribe();
        clearTimeout(timer);
        controller.signal.removeEventListener("abort", onAbort);
        if (cancellation.current === controller) cancellation.current = null;
        inFlight.current = false;
        if (mounted.current) {
          setPending(false);
          setLocalPending(false);
        }
      }
    },
    [
      getLocalContext,
      knownAgent,
      onCardOpened,
      onChatOpened,
      onWorktreeOpened,
      store,
    ],
  );

  const navigate = useCallback(
    (url: string, origin: InternalLinkOrigin = {}) =>
      runNavigation(url, origin),
    [runNavigation],
  );
  const navigateLocalTaskChat = useCallback(
    (chatId: string, taskId: string, cardId?: string, owner = false) => {
      const context = getLocalContext();
      const resolved =
        context && resolveChatLink(chatId, context.plannerChats, context.board);
      const trusted =
        context &&
        context.task.id === taskId &&
        resolved &&
        resolved.kind !== "unknown" &&
        (!cardId ||
          (resolved.kind === "agent" && resolved.cardId === cardId)) &&
        (!owner || context.board.owner_chat_id === chatId);
      return runNavigation(
        `refact://chat/${chatId}`,
        { taskId, cardId },
        trusted ? { context, owner } : undefined,
      );
    },
    [getLocalContext, runNavigation],
  );
  const navigateKnownAgent = useCallback(
    (chatId: string, sourceId: string) =>
      runNavigation(
        `refact://chat/${chatId}`,
        { chatId: sourceId },
        undefined,
        knownAgent(chatId, sourceId),
      ),
    [knownAgent, runNavigation],
  );
  return {
    navigate,
    navigateKnownAgent,
    navigateLocalTaskChat,
    pending,
    showProgress: pending && !localPending,
    error,
  };
}
