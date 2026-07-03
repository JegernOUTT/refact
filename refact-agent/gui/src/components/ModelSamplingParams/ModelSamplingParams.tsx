import React, { useMemo } from "react";
import { RotateCcw } from "lucide-react";

import { IconButton, FieldSlider, FieldSwitch, SegmentedControl } from "../ui";
import { useGetCapsQuery } from "../../services/refact/caps";
import type { ModelType } from "../../services/refact";
import { ReasoningIcon } from "../../features/Providers/ProviderForm/ProviderModelsList/components/CapabilityIcons";
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

export const ModelSamplingParams: React.FC<ModelSamplingParamsProps> = ({
  model,
  values,
  onChange,
  disabled = false,
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
      const contextWindow =
        completionModel?.n_ctx ?? capsData.code_completion_n_ctx;
      const maxTokens = Math.max(1024, Math.min(contextWindow, 16384));
      return {
        default_max_tokens: maxTokens,
        max_output_tokens: maxTokens,
      };
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
  const maxOutputTokens = modelDetail?.max_output_tokens ?? 16384;
  const reasoningEffortOptions = modelDetail?.reasoning_effort_options;
  const supportsThinkingBudget = modelDetail?.supports_thinking_budget ?? false;
  const supportsReasoning =
    capability === "chat" &&
    ((reasoningEffortOptions != null && reasoningEffortOptions.length > 0) ||
      supportsThinkingBudget);

  return (
    <div className={`${styles.container} rf-stagger`}>
      {supportsReasoning ? (
        <div className={`${styles.reasoningSection} rf-enter`}>
          <div className={styles.reasoningHeader}>
            <span className={styles.labelGroup}>
              <ReasoningIcon />
              Reasoning
            </span>
            <FieldSwitch
              checked={values.boost_reasoning ?? false}
              onChange={(checked) => {
                onChange("boost_reasoning", checked || undefined);
                if (!checked) {
                  onChange("reasoning_effort", undefined);
                  onChange("thinking_budget", undefined);
                }
              }}
              disabled={disabled}
            />
          </div>

          <div
            className="rf-expand-grid"
            data-open={values.boost_reasoning ? true : undefined}
          >
            <div>
              {values.boost_reasoning ? (
                <div className={styles.reasoningSection}>
                  {reasoningEffortOptions != null &&
                  reasoningEffortOptions.length > 0 ? (
                    <div className={styles.effortRow}>
                      <span className={styles.label}>Effort</span>
                      <SegmentedControl
                        className={styles.segmented}
                        size="sm"
                        value={values.reasoning_effort ?? "medium"}
                        onValueChange={(level) =>
                          onChange("reasoning_effort", level)
                        }
                        options={reasoningEffortOptions.map((level) => ({
                          value: level,
                          label: level,
                          disabled,
                        }))}
                      />
                    </div>
                  ) : null}

                  {supportsThinkingBudget ? (
                    <div className={styles.sliderRow}>
                      <div className={styles.sliderHeader}>
                        <span className={styles.label}>Thinking tokens</span>
                        <span className={styles.value}>
                          {values.thinking_budget ?? 16384}
                        </span>
                      </div>
                      <div className={styles.sliderTrack}>
                        <span className={styles.boundary}>1K</span>
                        <FieldSlider
                          className={styles.slider}
                          min={1024}
                          max={32768}
                          step={1024}
                          value={[values.thinking_budget ?? 16384]}
                          onChange={(v) => onChange("thinking_budget", v[0])}
                          disabled={disabled}
                          aria-label="Thinking tokens"
                        />
                        <span className={styles.boundary}>32K</span>
                      </div>
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}

      <div className={`${styles.sliderRow} rf-enter`}>
        <div className={styles.sliderHeader}>
          <span className={styles.label}>Max tokens</span>
          <span className={styles.valueGroup}>
            <span className={styles.value}>
              {values.max_new_tokens ?? `${defaultMaxTokens} (default)`}
            </span>
            {values.max_new_tokens != null ? (
              <IconButton
                className={styles.resetButton}
                icon={RotateCcw}
                size="sm"
                variant="plain"
                onClick={() => onChange("max_new_tokens", undefined)}
                disabled={disabled}
                aria-label="Reset max tokens"
              />
            ) : null}
          </span>
        </div>
        <div className={styles.sliderTrack}>
          <span className={styles.boundary}>1K</span>
          <FieldSlider
            className={styles.slider}
            min={1024}
            max={maxOutputTokens}
            step={1024}
            value={[values.max_new_tokens ?? defaultMaxTokens]}
            onChange={(v) => onChange("max_new_tokens", v[0])}
            disabled={disabled}
            aria-label="Max tokens"
          />
          <span className={styles.boundary}>
            {formatTokens(maxOutputTokens)}
          </span>
        </div>
      </div>
    </div>
  );
};
