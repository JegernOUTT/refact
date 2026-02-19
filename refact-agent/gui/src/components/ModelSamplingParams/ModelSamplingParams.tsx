import React, { useMemo } from "react";
import { Flex, Text, Slider, Switch } from "@radix-ui/themes";
import { useGetCapsQuery } from "../../services/refact/caps";
import styles from "./ModelSamplingParams.module.css";

export type SamplingValues = {
  temperature?: number;
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
};

function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(tokens % 1000000 === 0 ? 0 : 1)}M`;
  }
  return `${Math.round(tokens / 1000)}K`;
}

export const ModelSamplingParams: React.FC<ModelSamplingParamsProps> = ({
  model,
  values,
  onChange,
  disabled = false,
  size = "1",
}) => {
  const { data: capsData } = useGetCapsQuery(undefined);

  const modelDetail = useMemo(() => {
    if (!model || !capsData?.chat_models) return null;
    const m = capsData.chat_models[model] as
      | {
          n_ctx?: number;
          default_temperature?: number | null;
          default_max_tokens?: number | null;
          max_output_tokens?: number | null;
          reasoning_effort_options?: string[] | null;
          supports_thinking_budget?: boolean;
          supports_adaptive_thinking_budget?: boolean;
        }
      | undefined;
    return m ?? null;
  }, [model, capsData]);

  const defaultTemp = modelDetail?.default_temperature ?? 0.7;
  const defaultMaxTokens = modelDetail?.default_max_tokens ?? 4096;
  const maxOutputTokens = modelDetail?.max_output_tokens ?? 16384;
  const reasoningEffortOptions = modelDetail?.reasoning_effort_options;
  const supportsThinkingBudget = modelDetail?.supports_thinking_budget ?? false;
  const supportsReasoning =
    (reasoningEffortOptions != null && reasoningEffortOptions.length > 0) ||
    supportsThinkingBudget;

  const hasAnyReasoningConfigured =
    (values.boost_reasoning ?? false) ||
    values.reasoning_effort != null ||
    values.thinking_budget != null;

  return (
    <div className={styles.container}>
      {/* Reasoning */}
      {supportsReasoning && (
        <div className={styles.reasoningSection}>
          <Flex align="center" justify="between" gap="3">
            <Flex align="center" gap="1">
              <Text size={size}>🧠</Text>
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

      {/* Temperature */}
      <div className={styles.sliderRow}>
        <div className={styles.sliderHeader}>
          <Text size={size} color="gray">
            Temperature
          </Text>
          <Flex align="center" gap="2">
            <Text size={size} weight="medium">
              {hasAnyReasoningConfigured
                ? "None"
                : values.temperature?.toFixed(1) ??
                  `${defaultTemp.toFixed(1)} (default)`}
            </Text>
            {values.temperature != null && (
              <button
                type="button"
                className={styles.resetButton}
                onClick={() => onChange("temperature", undefined)}
                disabled={disabled || hasAnyReasoningConfigured}
              >
                ✕
              </button>
            )}
          </Flex>
        </div>
        <Slider
          size="1"
          min={0}
          max={2}
          step={0.1}
          value={[values.temperature ?? defaultTemp]}
          onValueChange={(v) => onChange("temperature", v[0])}
          disabled={disabled || hasAnyReasoningConfigured}
        />
      </div>

      {/* Max Tokens */}
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
              >
                ✕
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
            value={[values.max_new_tokens ?? defaultMaxTokens]}
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
