import { useState, useCallback } from "react";
import { useAppDispatch, useAppSelector } from "./index";
import { selectChatId } from "../features/Chat";
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
import { createChatWithId, switchToThread, requestSseRefresh } from "../features/Chat/Thread/actions";
import { push } from "../features/Pages/pagesSlice";

export type TrajectoryTab = "compress" | "handoff";

export function useTrajectoryOps() {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectChatId);

  const [activeTab, setActiveTab] = useState<TrajectoryTab>("compress");
  const [transformOptions, setTransformOptions] = useState<TransformOptions>({
    dedup_and_compress_context: true,
    drop_all_context: false,
    compress_non_agentic_tools: true,
  });
  const [handoffOptions, setHandoffOptions] = useState<HandoffOptions>({
    include_last_user_plus: false,
    include_all_opened_context: true,
    include_agentic_tools: true,
    llm_summary_for_excluded: false,
  });

  const [transformPreview, setTransformPreview] = useState<TransformPreviewResponse | null>(null);
  const [handoffPreview, setHandoffPreview] = useState<HandoffPreviewResponse | null>(null);

  const [previewTransform, { isLoading: isPreviewingTransform }] = usePreviewTransformMutation();
  const [applyTransform, { isLoading: isApplyingTransform }] = useApplyTransformMutation();
  const [previewHandoff, { isLoading: isPreviewingHandoff }] = usePreviewHandoffMutation();
  const [applyHandoff, { isLoading: isApplyingHandoff }] = useApplyHandoffMutation();

  const handlePreviewTransform = useCallback(async () => {
    if (!chatId) {
      console.error("[TrajectoryOps] No chatId available");
      return;
    }
    try {
      console.log("[TrajectoryOps] Previewing transform for chat:", chatId, "options:", transformOptions);
      const result = await previewTransform({ chatId, options: transformOptions }).unwrap();
      console.log("[TrajectoryOps] Transform preview result:", result);
      setTransformPreview(result);
    } catch (error) {
      console.error("[TrajectoryOps] Transform preview error:", error);
      setTransformPreview(null);
    }
  }, [chatId, transformOptions, previewTransform]);

  const handleApplyTransform = useCallback(async () => {
    if (!chatId) return false;
    try {
      console.log("[TrajectoryOps] Applying transform for chat:", chatId);
      const result = await applyTransform({ chatId, options: transformOptions }).unwrap();
      console.log("[TrajectoryOps] Transform apply result:", result);
      setTransformPreview(null);
      if (result.success) {
        console.log("[TrajectoryOps] Requesting SSE refresh to get updated snapshot");
        dispatch(requestSseRefresh({ chatId }));
      }
      return result.success;
    } catch (error) {
      console.error("[TrajectoryOps] Transform apply error:", error);
      return false;
    }
  }, [chatId, transformOptions, applyTransform, dispatch]);

  const handlePreviewHandoff = useCallback(async () => {
    if (!chatId) {
      console.error("[TrajectoryOps] No chatId available for handoff");
      return;
    }
    try {
      console.log("[TrajectoryOps] Previewing handoff for chat:", chatId, "options:", handoffOptions);
      const result = await previewHandoff({ chatId, options: handoffOptions }).unwrap();
      console.log("[TrajectoryOps] Handoff preview result:", result);
      setHandoffPreview(result);
    } catch (error) {
      console.error("[TrajectoryOps] Handoff preview error:", error);
      setHandoffPreview(null);
    }
  }, [chatId, handoffOptions, previewHandoff]);

  const handleApplyHandoff = useCallback(async () => {
    if (!chatId) return false;
    try {
      const result = await applyHandoff({ chatId, options: handoffOptions }).unwrap();
      if (result.success && result.new_chat_id) {
        dispatch(createChatWithId({ id: result.new_chat_id }));
        dispatch(switchToThread({ id: result.new_chat_id }));
        dispatch(push({ name: "chat" }));
        setHandoffPreview(null);
        return true;
      }
      return false;
    } catch {
      return false;
    }
  }, [chatId, handoffOptions, applyHandoff, dispatch]);

  const clearPreviews = useCallback(() => {
    setTransformPreview(null);
    setHandoffPreview(null);
  }, []);

  const updateTransformOption = useCallback((key: keyof TransformOptions, value: boolean) => {
    setTransformOptions((prev) => ({ ...prev, [key]: value }));
    setTransformPreview(null);
  }, []);

  const updateHandoffOption = useCallback((key: keyof HandoffOptions, value: boolean) => {
    setHandoffOptions((prev) => ({ ...prev, [key]: value }));
    setHandoffPreview(null);
  }, []);

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
