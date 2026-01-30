import { useCallback, useEffect, useMemo, useState } from "react";
import { selectThreadMode } from "../features/Chat/Thread/selectors";
import { useAppSelector, useGetCapsQuery, useAppDispatch } from ".";
import { useGetChatModesQuery } from "../services/refact/chatModes";

import {
  getSelectedChatModel,
  setChatModel,
  setMaxNewTokens,
} from "../features/Chat";

export const PAID_AGENT_LIST = [
  "gpt-4o",
  "claude-3-5-sonnet",
  "grok-2-1212",
  "grok-beta",
  "gemini-2.0-flash-exp",
  "claude-3-7-sonnet",
];

export const UNLIMITED_PRO_MODELS_LIST = ["gpt-4o-mini"];

export function useCapsForToolUse() {
  const [wasAdjusted, setWasAdjusted] = useState(false);
  const caps = useGetCapsQuery();
  const modesQuery = useGetChatModesQuery(undefined);
  const currentMode = useAppSelector(selectThreadMode);
  const dispatch = useAppDispatch();

  const defaultCap = caps.data?.chat_default_model ?? "";
  const selectedModel = useAppSelector(getSelectedChatModel);
  const currentModel = selectedModel || defaultCap;

  const modeInfo = useMemo(() => {
    if (!modesQuery.data?.modes) return null;
    return modesQuery.data.modes.find((m) => m.id === currentMode) ?? null;
  }, [modesQuery.data?.modes, currentMode]);

  const modeRequiresTools = useMemo(() => {
    if (!modeInfo) return false;
    return modeInfo.tools_count > 0;
  }, [modeInfo]);

  const modeRequiresAgent = useMemo(() => {
    if (!modeInfo) return false;
    return modeInfo.ui.tags.includes("editing");
  }, [modeInfo]);

  const setCapModel = useCallback(
    (value: string) => {
      const model =
        caps.data?.chat_default_model === value
          ? caps.data.chat_default_model
          : value;
      const action = setChatModel(model);
      dispatch(action);
      const tokens = caps.data?.chat_models[value]?.n_ctx;
      if (tokens !== undefined) {
        dispatch(setMaxNewTokens(tokens));
      }
    },
    [caps.data?.chat_default_model, caps.data?.chat_models, dispatch],
  );

  const isMultimodalitySupportedForCurrentModel = useMemo(() => {
    const models = caps.data?.chat_models;
    const item = models?.[currentModel];
    if (!item) return false;
    if (!item.supports_multimodality) return false;
    return true;
  }, [caps.data?.chat_models, currentModel]);

  const modelsSupportingTools = useMemo(() => {
    const models = caps.data?.chat_models ?? {};
    return Object.entries(models)
      .filter(([_, value]) => value.supports_tools)
      .map(([key]) => key);
  }, [caps.data?.chat_models]);

  const modelsSupportingAgent = useMemo(() => {
    const models = caps.data?.chat_models ?? {};
    return Object.entries(models)
      .filter(([_, value]) => value.supports_agent)
      .map(([key]) => key);
  }, [caps.data?.chat_models]);

  const usableModels = useMemo(() => {
    const models = caps.data?.chat_models ?? {};
    const items = Object.entries(models).reduce<string[]>(
      (acc, [key, value]) => {
        if (modeRequiresAgent && !value.supports_agent) return acc;
        if (modeRequiresTools && !value.supports_tools) return acc;
        return [...acc, key];
      },
      [],
    );
    return items;
  }, [caps.data?.chat_models, modeRequiresAgent, modeRequiresTools]);

  const usableModelsForPlan = useMemo(() => {
    return usableModels.map((model) => {
      return {
        value: model,
        disabled: false,
        textValue: model,
      };
    });
  }, [usableModels]);

  useEffect(() => {
    if (usableModelsForPlan.length > 0) {
      const models: string[] = usableModelsForPlan.map(
        (elem) => elem.textValue,
      );
      const toChange =
        models.find((elem) => currentModel === elem) ?? models[0];

      if (toChange && toChange !== currentModel) {
        setCapModel(toChange);
      }
    }
  }, [setCapModel, currentModel, usableModels, usableModelsForPlan]);

  useEffect(() => {
    if (!caps.isSuccess || wasAdjusted) return;
    setWasAdjusted(true);
  }, [caps.isSuccess, wasAdjusted]);

  return {
    usableModels,
    usableModelsForPlan,
    currentModel,
    setCapModel,
    isMultimodalitySupportedForCurrentModel,
    loading: !caps.data && (caps.isFetching || caps.isLoading),
    uninitialized: caps.isUninitialized,
    data: caps.data,
    modelsSupportingTools,
    modelsSupportingAgent,
    modeRequiresTools,
    modeRequiresAgent,
  };
}
