import React, { useMemo } from "react";
import { Flex, Text, Slider, Switch } from "@radix-ui/themes";
import { Cross1Icon } from "@radix-ui/react-icons";
import { useGetCapsQuery } from "../../services/refact/caps";
import { ReasoningIcon } from "../../features/Providers/ProviderForm/ProviderModelsList/components/CapabilityIcons";
import type { ModelType } from "../../services/refact";
import styles from "./ModelSamplingParams.module.css";

export type SamplingValues = {
  max_new_tokens?: number;
  top_p?: number;
  boost_reasoning?: boolean;
  reasoning_effort?: string;
  thinking_budget?: number;
};

type ModelSamplingParamsProps = {
  model: string | undefined;
  values: SamplingValues;
  onChange: <K extends keyof SamplingValues>(
    field: K,
    value: SamplingValues[K],
  ) => void;
  disabled?: boolean;
  size?: "1" | "2";
  capability?: ModelType;
};

type SamplingModelDetail = {
  default_max_tokens?: number | null;
  max_output_tokens?: number | null;
  reasoning_effort_options?: string[] | null;
  supports_thinking_budget?: boolean;
};

function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(tokens % 1000000 === 0 ? 0 : 1)}M`;
  }
  return `${Math.round(tokens / 1000)}K`;
}

function completionDefaults(nCtx: number | undefined): SamplingModelDetail {
  const contextWindow = nCtx ?? 16384;
  const maxTokens = Math.max(1024, Math.min(contextWindow, 16384));
  return {
    default_max_tokens: maxTokens,
    max_output_tokens: maxTokens,
  };
}

export const ModelSamplingParams: React.FC<ModelSamplingParamsProps> = ({
  model,
  values,
  onChange,
  disabled = false,
  size = "1",
  capability = "chat",
}) => {
  const { data: capsData } = useGetCapsQuery(undefined);

  const modelDetail = useMemo<SamplingModelDetail | null>(() => {
    if (capability === "embedding") return null;
    if (!model || !capsData) return null;

    if (capability === "completion") {
      const completionModel = Object.entries(capsData.completion_models).find(
        ([name]) => name === model,
      )?.[1];
      return completionDefaults(
        completionModel?.n_ctx ?? capsData.code_completion_n_ctx,
      );
    }

    return (
      Object.entries(capsData.chat_models).find(
        ([name]) => name === model,
      )?.[1] ?? null
    );
  }, [model, capsData, capability]);

  if (capability === "embedding") {
    return null;
  }

  const defaultMaxTokens = modelDetail?.default_max_tokens ?? 4096;
  const maxOutputTokens = Math.max(
    1024,
    modelDetail?.max_output_tokens ?? defaultMaxTokens,
  );
  const sliderValue = Math.min(
    values.max_new_tokens ?? defaultMaxTokens,
    maxOutputTokens,
  );
  const reasoningEffortOptions = modelDetail?.reasoning_effort_options;
  const supportsThinkingBudget = modelDetail?.supports_thinking_budget ?? false;
  const supportsReasoning =
    capability === "chat" &&
    ((reasoningEffortOptions != null && reasoningEffortOptions.length > 0) ||
      supportsThinkingBudget);

  return (
    <div className={styles.container}>
      {supportsReasoning && (
        <div className={styles.reasoningSection}>
          <Flex align="center" justify="between" gap="3">
            <Flex align="center" gap="1">
              <Text size={size}>
                <ReasoningIcon />
              </Text>
              <Text size={size} weight="medium">
                Reasoning
              </Text>
            </Flex>
            <Switch
              size="1"
              checked={values.boost_reasoning ?? false}
              onCheckedChange={(checked) => {
                onChange("boost_reasoning", checked || undefined);
                if (!checked) {
                  onChange("reasoning_effort", undefined);
                  onChange("thinking_budget", undefined);
                }
              }}
              disabled={disabled}
            />
          </Flex>

          {values.boost_reasoning && (
            <>
              {reasoningEffortOptions != null &&
                reasoningEffortOptions.length > 0 && (
                  <div className={styles.effortRow}>
                    <Text size={size} color="gray">
                      Effort
                    </Text>
                    <div className={styles.effortButtons}>
                      {reasoningEffortOptions.map((level) => (
                        <button
                          key={level}
                          type="button"
                          className={`${styles.effortButton} ${
                            (values.reasoning_effort ?? "medium") === level
                              ? styles.effortButtonActive
                              : ""
                          }`}
                          onClick={() => onChange("reasoning_effort", level)}
                          disabled={disabled}
                        >
                          <Text size={size}>{level}</Text>
                        </button>
                      ))}
                    </div>
                  </div>
                )}

              {supportsThinkingBudget && (
                <div className={styles.sliderRow}>
                  <div className={styles.sliderHeader}>
                    <Text size={size} color="gray">
                      Thinking tokens
                    </Text>
                    <Text size={size} weight="medium">
                      {values.thinking_budget ?? 16384}
                    </Text>
                  </div>
                  <div className={styles.sliderTrack}>
                    <Text size="1" color="gray">
                      1K
                    </Text>
                    <Slider
                      size="1"
                      min={1024}
                      max={32768}
                      step={1024}
                      value={[values.thinking_budget ?? 16384]}
                      onValueChange={(v) => onChange("thinking_budget", v[0])}
                      disabled={disabled}
                      className={styles.slider}
                    />
                    <Text size="1" color="gray">
                      32K
                    </Text>
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      )}

      <div className={styles.sliderRow}>
        <div className={styles.sliderHeader}>
          <Text size={size} color="gray">
            Max tokens
          </Text>
          <Flex align="center" gap="2">
            <Text size={size} weight="medium">
              {values.max_new_tokens ?? `${defaultMaxTokens} (default)`}
            </Text>
            {values.max_new_tokens != null && (
              <button
                type="button"
                className={styles.resetButton}
                onClick={() => onChange("max_new_tokens", undefined)}
                disabled={disabled}
                aria-label="Reset max tokens"
              >
                <Cross1Icon />
              </button>
            )}
          </Flex>
        </div>
        <div className={styles.sliderTrack}>
          <Text size="1" color="gray">
            1K
          </Text>
          <Slider
            size="1"
            min={1024}
            max={maxOutputTokens}
            step={1024}
            value={[sliderValue]}
            onValueChange={(v) => onChange("max_new_tokens", v[0])}
            disabled={disabled}
            className={styles.slider}
          />
          <Text size="1" color="gray">
            {formatTokens(maxOutputTokens)}
          </Text>
        </div>
      </div>
    </div>
  );
};
