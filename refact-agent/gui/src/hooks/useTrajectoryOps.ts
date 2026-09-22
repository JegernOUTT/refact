import { useState, useCallback, useEffect, useRef } from "react";
import { useAppDispatch, useAppSelector } from "./index";
import { selectChatId } from "../features/Chat";
import {
  usePreviewTransformMutation,
  useApplyTransformMutation,
  usePreviewLlmCompressMutation,
  useApplyLlmCompressMutation,
  TransformOptions,
  TransformPreviewResponse,
  LlmCompressOptions,
  LlmCompressPreviewResponse,
} from "../services/refact/trajectory";
import { trajectoriesApi } from "../services/refact/trajectories";
import { requestSseRefresh } from "../features/Chat/Thread/actions";

export type TrajectoryTab = "compress" | "llm-compress";

export function useTrajectoryOps(
  initialTab: TrajectoryTab = "compress",
  targetChatId?: string,
) {
  const dispatch = useAppDispatch();
  const currentChatId = useAppSelector(selectChatId);
  const chatId = targetChatId ?? currentChatId;
  const [activeTab, setActiveTab] = useState<TrajectoryTab>(initialTab);
  const [transformOptions, setTransformOptions] = useState<TransformOptions>({
    dedup_and_compress_context: true,
    drop_all_context: false,
    compress_non_agentic_tools: true,
    drop_all_memories: false,
    drop_project_information: false,
    strip_metering: false,
  });
  const [llmCompressOptions, setLlmCompressOptions] =
    useState<LlmCompressOptions>({});

  const [transformPreview, setTransformPreview] =
    useState<TransformPreviewResponse | null>(null);
  const [llmCompressPreview, setLlmCompressPreview] =
    useState<LlmCompressPreviewResponse | null>(null);
  const [llmCompressError, setLlmCompressError] = useState<string | null>(null);
  const llmPreviewRequestId = useRef(0);
  const llmPreviewChatId = useRef(chatId);
  llmPreviewChatId.current = chatId;

  useEffect(() => {
    llmPreviewRequestId.current += 1;
    setLlmCompressPreview(null);
    setLlmCompressError(null);
  }, [chatId]);

  const [previewTransform, { isLoading: isPreviewingTransform }] =
    usePreviewTransformMutation();
  const [applyTransform, { isLoading: isApplyingTransform }] =
    useApplyTransformMutation();
  const [previewLlmCompress, { isLoading: isPreviewingLlmCompress }] =
    usePreviewLlmCompressMutation();
  const [applyLlmCompress, { isLoading: isApplyingLlmCompress }] =
    useApplyLlmCompressMutation();

  const handlePreviewLlmCompress = useCallback(async () => {
    const previewChatId = chatId;
    if (!previewChatId) return;
    const requestId = ++llmPreviewRequestId.current;
    setLlmCompressError(null);
    try {
      const result = await previewLlmCompress({
        chatId: previewChatId,
        options: llmCompressOptions,
      }).unwrap();
      if (
        requestId === llmPreviewRequestId.current &&
        previewChatId === llmPreviewChatId.current
      ) {
        setLlmCompressPreview(result);
      }
    } catch {
      if (
        requestId === llmPreviewRequestId.current &&
        previewChatId === llmPreviewChatId.current
      ) {
        setLlmCompressPreview(null);
        setLlmCompressError(
          "Unable to preview LLM compression. Please try again.",
        );
      }
    }
  }, [chatId, llmCompressOptions, previewLlmCompress]);

  const handleApplyLlmCompress = useCallback(async () => {
    if (!chatId || !llmCompressPreview) return false;
    setLlmCompressError(null);
    try {
      const result = await applyLlmCompress({
        chatId,
        options: {
          ...llmCompressOptions,
          expected_trajectory_version: llmCompressPreview.trajectory_version,
        },
      }).unwrap();
      if (!result.applied) {
        setLlmCompressError(
          result.reason ?? "LLM compression was not applied.",
        );
        return false;
      }
      setLlmCompressPreview(null);
      dispatch(requestSseRefresh({ chatId }));
      void dispatch(
        trajectoriesApi.endpoints.listAllTrajectories.initiate(undefined, {
          forceRefetch: true,
        }),
      );
      return true;
    } catch {
      setLlmCompressError("Unable to apply LLM compression. Please try again.");
      return false;
    }
  }, [
    applyLlmCompress,
    chatId,
    dispatch,
    llmCompressOptions,
    llmCompressPreview,
  ]);

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

  const clearPreviews = useCallback(() => {
    llmPreviewRequestId.current += 1;
    setTransformPreview(null);
    setLlmCompressPreview(null);
    setLlmCompressError(null);
  }, []);

  const updateTransformOption = useCallback(
    (key: keyof TransformOptions, value: boolean) => {
      setTransformOptions((prev) => ({ ...prev, [key]: value }));
      setTransformPreview(null);
    },
    [],
  );

  const updateLlmCompressModel = useCallback((summaryModel?: string) => {
    llmPreviewRequestId.current += 1;
    setLlmCompressOptions(summaryModel ? { summary_model: summaryModel } : {});
    setLlmCompressPreview(null);
    setLlmCompressError(null);
  }, []);

  return {
    chatId,
    activeTab,
    setActiveTab,
    transformOptions,
    llmCompressOptions,
    transformPreview,
    llmCompressPreview,
    llmCompressError,
    isPreviewingTransform,
    isApplyingTransform,
    isPreviewingLlmCompress,
    isApplyingLlmCompress,
    handlePreviewTransform,
    handleApplyTransform,
    handlePreviewLlmCompress,
    handleApplyLlmCompress,
    clearPreviews,
    updateTransformOption,
    updateLlmCompressModel,
  };
}
