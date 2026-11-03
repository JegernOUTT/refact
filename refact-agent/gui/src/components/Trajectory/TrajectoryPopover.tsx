import React from "react";
import { Popover } from "../LongTailPrimitives";
import {
  KitButton,
  KitCheckRow,
  KitDivider,
  KitFooter,
  KitHeader,
  KitKeyValue,
  KitNote,
  KitPanel,
  KitSection,
  KitStaticRow,
  KitTabs,
  kit,
} from "../ComposerKit";
import {
  useTrajectoryOps,
  type TrajectoryTab,
} from "../../hooks/useTrajectoryOps";
import { useCapsForToolUse } from "../../hooks/useCapsForToolUse";
import { ModelSelector } from "../Chat/ModelSelector";
import { formatContextWindow } from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import styles from "./TrajectoryPopover.module.css";

type TrajectoryPopoverContentProps = {
  onClose: () => void;
  initialTab?: TrajectoryTab;
  chatId?: string;
};

export const TrajectoryPopoverContent: React.FC<
  TrajectoryPopoverContentProps
> = ({ onClose, initialTab, chatId }) => {
  const {
    isStreaming,
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
  } = useTrajectoryOps(initialTab, chatId);
  const caps = useCapsForToolUse();

  const handleTabChange = (value: string) => {
    setActiveTab(value as "compress" | "llm-compress");
    clearPreviews();
  };

  const handleApplyTransformClick = async () => {
    const success = await handleApplyTransform();
    if (success) {
      onClose();
    }
  };

  const handleApplyLlmCompressClick = async () => {
    const success = await handleApplyLlmCompress();
    if (success) {
      onClose();
    }
  };

  const reduction =
    transformPreview && transformPreview.stats.before_approx_tokens > 0
      ? Math.round(
          ((transformPreview.stats.before_approx_tokens -
            transformPreview.stats.after_approx_tokens) /
            transformPreview.stats.before_approx_tokens) *
            100,
        )
      : 0;

  const inPlace = (
    <>
      <KitCheckRow
        label="Drop all context files"
        checked={transformOptions.drop_all_context === true}
        onCheckedChange={(enabled) => {
          updateTransformOption("drop_all_context", enabled);
          if (enabled) {
            updateTransformOption("dedup_and_compress_context", false);
          }
        }}
      />
      <KitCheckRow
        label="Deduplicate context files"
        hint="Keeps the latest copy of each file"
        checked={transformOptions.dedup_and_compress_context === true}
        disabled={transformOptions.drop_all_context}
        onCheckedChange={(enabled) =>
          updateTransformOption("dedup_and_compress_context", enabled)
        }
      />
      <KitCheckRow
        label="Truncate tool results"
        checked={transformOptions.compress_non_agentic_tools === true}
        onCheckedChange={(enabled) =>
          updateTransformOption("compress_non_agentic_tools", enabled)
        }
      />
      <KitCheckRow
        label="Drop all memories"
        checked={transformOptions.drop_all_memories === true}
        onCheckedChange={(enabled) =>
          updateTransformOption("drop_all_memories", enabled)
        }
      />
      <KitCheckRow
        label="Drop project information"
        checked={transformOptions.drop_project_information === true}
        onCheckedChange={(enabled) =>
          updateTransformOption("drop_project_information", enabled)
        }
      />
      <KitCheckRow
        label="Remove usage and metering details"
        checked={transformOptions.strip_metering === true}
        onCheckedChange={(enabled) =>
          updateTransformOption("strip_metering", enabled)
        }
      />
      {transformPreview ? (
        <>
          <KitDivider />
          <KitKeyValue label="Estimated reduction" value={`~${reduction}%`} />
          {transformPreview.actions.map((action, index) => (
            <KitNote key={index}>{action}</KitNote>
          ))}
        </>
      ) : null}
      <KitDivider />
      <KitFooter>
        <KitButton
          disabled={isStreaming || isPreviewingTransform}
          onClick={() => {
            void handlePreviewTransform();
          }}
        >
          {isPreviewingTransform ? "Previewing…" : "Preview"}
        </KitButton>
        <KitButton
          variant="primary"
          disabled={isStreaming || !transformPreview || isApplyingTransform}
          onClick={() => {
            void handleApplyTransformClick();
          }}
        >
          {isApplyingTransform ? "Applying…" : "Apply"}
        </KitButton>
      </KitFooter>
    </>
  );

  const llmSummary = (
    <>
      <KitNote>
        Rebuilds the active conversation context with an LLM. Original chat
        messages remain visible in the transcript. The selected provider
        receives only content allowed by your privacy policy.
      </KitNote>
      <KitSection>Summary model</KitSection>
      {caps.usableModels.length > 0 ? (
        <div className={styles.modelPicker}>
          <ModelSelector
            value={llmCompressOptions.summary_model}
            defaultValue=""
            onValueChange={(model) =>
              updateLlmCompressModel(model || undefined)
            }
            allowUnset
            unsetLabel="Use automatic summary model"
            showLabel={false}
            compact={false}
          />
        </div>
      ) : (
        <KitStaticRow
          tone="muted"
          label="Configure an eligible chat model in Providers before using LLM compression."
        />
      )}
      {!llmCompressOptions.summary_model && caps.currentModel ? (
        <KitNote>Current chat model: {caps.currentModel}</KitNote>
      ) : null}
      {llmCompressPreview ? (
        <>
          <KitDivider />
          <KitStaticRow
            label={
              llmCompressPreview.eligible
                ? `${
                    llmCompressPreview.source_messages
                  } messages (~${llmCompressPreview.approximate_source_tokens.toLocaleString()} tokens) eligible`
                : "Context rebuild is not currently available"
            }
            strong
          />
          {llmCompressPreview.resolved_model ? (
            <KitKeyValue
              label="Model"
              value={llmCompressPreview.resolved_model}
            />
          ) : null}
          {llmCompressPreview.context_window != null ? (
            <KitKeyValue
              label="Context window"
              value={formatContextWindow(llmCompressPreview.context_window)}
            />
          ) : null}
          {llmCompressPreview.reason ? (
            <KitStaticRow tone="danger" label={llmCompressPreview.reason} />
          ) : null}
        </>
      ) : null}
      {llmCompressError ? (
        <div role="alert">
          <KitStaticRow tone="danger" label={llmCompressError} />
        </div>
      ) : null}
      <KitDivider />
      <KitFooter>
        <KitButton
          disabled={
            isStreaming ||
            caps.usableModels.length === 0 ||
            isPreviewingLlmCompress
          }
          onClick={() => {
            void handlePreviewLlmCompress();
          }}
        >
          {isPreviewingLlmCompress ? "Previewing…" : "Preview"}
        </KitButton>
        <KitButton
          variant="primary"
          disabled={
            isStreaming ||
            !llmCompressPreview?.eligible ||
            isApplyingLlmCompress
          }
          onClick={() => {
            void handleApplyLlmCompressClick();
          }}
        >
          {isApplyingLlmCompress ? "Rebuilding…" : "Rebuild context"}
        </KitButton>
      </KitFooter>
    </>
  );

  return (
    <Popover.Content
      side="top"
      align="end"
      sideOffset={8}
      className={kit.popover}
      maxWidth="min(340px, calc(100vw - 2 * var(--rf-space-3)))"
      maxHeight="min(520px, calc(100dvh - var(--rf-space-5)))"
    >
      <KitPanel>
        <KitHeader title="Compress context" />
        <KitTabs
          value={activeTab}
          onValueChange={handleTabChange}
          label="Compression method"
          tabs={[
            { value: "compress", label: "In place", content: inPlace },
            {
              value: "llm-compress",
              label: "LLM summary",
              content: llmSummary,
            },
          ]}
        />
      </KitPanel>
    </Popover.Content>
  );
};
