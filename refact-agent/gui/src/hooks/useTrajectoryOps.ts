import { useState, useCallback } from "react";
import { useAppDispatch, useAppSelector } from "./index";
import { selectChatId, selectThread } from "../features/Chat";
import {
  usePreviewTransformMutation,
  useApplyTransformMutation,
  usePreviewHandoffMutation,
  useApplyHandoffMutation,
  TransformOptions,
  HandoffOptions,
  TransformPreviewResponse,
  HandoffPreviewResponse,
} from "../services/refact/trajectory";
import { trajectoriesApi } from "../services/refact/trajectories";
import {
  createChatWithId,
  requestSseRefresh,
  closeThread,
} from "../features/Chat/Thread/actions";
import {
  selectBrowserRuntime,
  setBrowserRuntime,
  removeBrowserRuntime,
} from "../features/Browser/browserSlice";
import { push } from "../features/Pages/pagesSlice";
import { selectLspPort, selectApiKey } from "../features/Config/configSlice";
import { regenerate } from "../services/refact/chatCommands";

export type TrajectoryTab = "compress" | "handoff";

export function useTrajectoryOps() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);
  const thread = useAppSelector(selectThread);
  const browserRuntime = useAppSelector((state) =>
    chatId ? selectBrowserRuntime(state, chatId) : undefined,
  );
  const port = useAppSelector(selectLspPort);
  const apiKey = useAppSelector(selectApiKey);

  const [activeTab, setActiveTab] = useState<TrajectoryTab>("compress");
  const [transformOptions, setTransformOptions] = useState<TransformOptions>({
    dedup_and_compress_context: true,
    drop_all_context: false,
    compress_non_agentic_tools: true,
    drop_all_memories: false,
    drop_project_information: false,
  });
  const [handoffOptions, setHandoffOptions] = useState<HandoffOptions>({
    include_last_user_plus: false,
    include_all_opened_context: false,
    include_all_edited_context: false,
    include_agentic_tools: false,
    llm_summary_for_excluded: true,
    include_all_user_assistant_only: false,
  });

  const [transformPreview, setTransformPreview] =
    useState<TransformPreviewResponse | null>(null);
  const [handoffPreview, setHandoffPreview] =
    useState<HandoffPreviewResponse | null>(null);

  const [previewTransform, { isLoading: isPreviewingTransform }] =
    usePreviewTransformMutation();
  const [applyTransform, { isLoading: isApplyingTransform }] =
    useApplyTransformMutation();
  const [previewHandoff, { isLoading: isPreviewingHandoff }] =
    usePreviewHandoffMutation();
  const [applyHandoff, { isLoading: isApplyingHandoff }] =
    useApplyHandoffMutation();

  const handlePreviewTransform = useCallback(async () => {
    if (!chatId) return;
    try {
      const result = await previewTransform({
        chatId,
        options: transformOptions,
      }).unwrap();
      setTransformPreview(result);
    } catch {
      setTransformPreview(null);
    }
  }, [chatId, transformOptions, previewTransform]);

  const handleApplyTransform = useCallback(async () => {
    if (!chatId) return false;
    try {
      await applyTransform({ chatId, options: transformOptions }).unwrap();
      setTransformPreview(null);
      dispatch(requestSseRefresh({ chatId }));
      return true;
    } catch {
      return false;
    }
  }, [chatId, transformOptions, applyTransform, dispatch]);

  const handlePreviewHandoff = useCallback(async () => {
    if (!chatId) return;
    try {
      const result = await previewHandoff({
        chatId,
        options: handoffOptions,
      }).unwrap();
      setHandoffPreview(result);
    } catch {
      setHandoffPreview(null);
    }
  }, [chatId, handoffOptions, previewHandoff]);

  const handleApplyHandoff = useCallback(async () => {
    if (!chatId || !thread) return false;
    try {
      const isTaskChat = thread.is_task_chat;
      const taskMeta = thread.task_meta;
      const oldChatId = chatId;

      const result = await applyHandoff({
        chatId,
        options: handoffOptions,
      }).unwrap();

      await dispatch(
        trajectoriesApi.endpoints.listAllTrajectories.initiate(undefined, {
          forceRefetch: true,
        }),
      );

      // Transfer browser runtime state to the new chat if one was active
      if (browserRuntime && result.browser_runtime_id) {
        dispatch(
          setBrowserRuntime({
            chatId: result.new_chat_id,
            runtime: {
              ...browserRuntime,
              runtime_id: result.browser_runtime_id,
              notification: {
                type: "attached",
                message: "Browser session transferred via handoff",
              },
            },
          }),
        );
        dispatch(removeBrowserRuntime({ chatId: oldChatId }));
      }

      if (isTaskChat && taskMeta?.role === "planner") {
        const taskId = taskMeta.task_id;
        const now = new Date().toISOString();

        dispatch(closeThread({ id: oldChatId, force: true }));

        dispatch(
          createChatWithId({
            id: result.new_chat_id,
            title: "",
            isTaskChat: true,
            mode: "TASK_PLANNER",
            taskMeta: { task_id: taskId, role: "planner" },
          }),
        );

        const { addPlannerChat, setTaskActiveChat } = await import(
          "../features/Tasks/tasksSlice"
        );

        dispatch(
          addPlannerChat({
            taskId,
            planner: {
              id: result.new_chat_id,
              title: "",
              createdAt: now,
              updatedAt: now,
            },
          }),
        );

        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: { type: "planner", chatId: result.new_chat_id },
          }),
        );

        dispatch(requestSseRefresh({ chatId: result.new_chat_id }));
        setHandoffPreview(null);
        try {
          await regenerate(result.new_chat_id, port, apiKey ?? undefined);
        } catch {
          // regenerate failure is non-critical; handoff was applied successfully
        }
      } else {
        dispatch(closeThread({ id: oldChatId, force: true }));
        dispatch(createChatWithId({ id: result.new_chat_id }));
        dispatch(requestSseRefresh({ chatId: result.new_chat_id }));
        dispatch(push({ name: "chat" }));
        setHandoffPreview(null);
        try {
          await regenerate(result.new_chat_id, port, apiKey ?? undefined);
        } catch {
          // regenerate failure is non-critical; handoff was applied successfully
        }
      }

      return true;
    } catch (error) {
      // eslint-disable-next-line no-console
      console.error("[handleApplyHandoff]", error);
      return false;
    }
  }, [
    chatId,
    thread,
    browserRuntime,
    handoffOptions,
    applyHandoff,
    dispatch,
    port,
    apiKey,
  ]);

  const clearPreviews = useCallback(() => {
    setTransformPreview(null);
    setHandoffPreview(null);
  }, []);

  const updateTransformOption = useCallback(
    (key: keyof TransformOptions, value: boolean) => {
      setTransformOptions((prev) => ({ ...prev, [key]: value }));
      setTransformPreview(null);
    },
    [],
  );

  const updateHandoffOption = useCallback(
    (key: keyof HandoffOptions, value: boolean) => {
      setHandoffOptions((prev) => ({ ...prev, [key]: value }));
      setHandoffPreview(null);
    },
    [],
  );

  return {
    chatId,
    activeTab,
    setActiveTab,
    transformOptions,
    handoffOptions,
    transformPreview,
    handoffPreview,
    isPreviewingTransform,
    isApplyingTransform,
    isPreviewingHandoff,
    isApplyingHandoff,
    handlePreviewTransform,
    handleApplyTransform,
    handlePreviewHandoff,
    handleApplyHandoff,
    clearPreviews,
    updateTransformOption,
    updateHandoffOption,
  };
}
