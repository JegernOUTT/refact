import React from "react";
import { Box, Flex, Popover, Text } from "../LongTailPrimitives";
import { Checkbox } from "../Checkbox";
import { Button, Tabs } from "../ui";
import {
  useTrajectoryOps,
  type TrajectoryTab,
} from "../../hooks/useTrajectoryOps";
import { useCapsForToolUse } from "../../hooks/useCapsForToolUse";
import { ModelSelector } from "../Chat/ModelSelector";
import { formatContextWindow } from "../../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import styles from "./TrajectoryPopover.module.css";

const TAB_OPTIONS = [
  { value: "compress", label: "Compress in-place" },
  { value: "llm-compress", label: "LLM compression" },
];

type TrajectoryPopoverContentProps = {
  onClose: () => void;
  initialTab?: TrajectoryTab;
  chatId?: string;
};

export const TrajectoryPopoverContent: React.FC<
  TrajectoryPopoverContentProps
> = ({ onClose, initialTab, chatId }) => {
  const {
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

  const activeTabIndex = TAB_OPTIONS.findIndex(
    (tab) => tab.value === activeTab,
  );

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

  return (
    <Popover.Content
      side="bottom"
      align="end"
      sideOffset={8}
      className={styles.popoverContent}
      maxWidth="min(360px, calc(100vw - 2 * var(--rf-space-3)))"
      maxHeight="min(520px, calc(100dvh - var(--rf-space-5)))"
    >
      <Tabs value={activeTab} onValueChange={handleTabChange}>
        <Tabs.List
          activeIndex={activeTabIndex < 0 ? 0 : activeTabIndex}
          itemCount={TAB_OPTIONS.length}
          className={styles.tabStrip}
        >
          {TAB_OPTIONS.map((tab) => (
            <Tabs.Trigger key={tab.value} value={tab.value}>
              {tab.label}
            </Tabs.Trigger>
          ))}
        </Tabs.List>

        <Tabs.Content value="compress">
          <div className={styles.optionsSection}>
            <Checkbox
              checked={transformOptions.drop_all_context}
              onCheckedChange={(checked) => {
                const enabled = checked === true;
                updateTransformOption("drop_all_context", enabled);
                if (enabled) {
                  updateTransformOption("dedup_and_compress_context", false);
                }
              }}
            >
              Drop all context files
            </Checkbox>
            <div className={styles.nestedOption}>
              <Checkbox
                checked={transformOptions.dedup_and_compress_context}
                disabled={transformOptions.drop_all_context}
                onCheckedChange={(checked) =>
                  updateTransformOption(
                    "dedup_and_compress_context",
                    checked === true,
                  )
                }
              >
                Deduplicate context files
              </Checkbox>
            </div>
            <Checkbox
              checked={transformOptions.compress_non_agentic_tools}
              onCheckedChange={(checked) =>
                updateTransformOption(
                  "compress_non_agentic_tools",
                  checked === true,
                )
              }
            >
              Truncate tool results
            </Checkbox>
            <Checkbox
              checked={transformOptions.drop_all_memories}
              onCheckedChange={(checked) =>
                updateTransformOption("drop_all_memories", checked === true)
              }
            >
              Drop all memories
            </Checkbox>
            <Checkbox
              checked={transformOptions.drop_project_information}
              onCheckedChange={(checked) =>
                updateTransformOption(
                  "drop_project_information",
                  checked === true,
                )
              }
            >
              Drop project information
            </Checkbox>
            <Checkbox
              checked={transformOptions.strip_metering}
              onCheckedChange={(checked) =>
                updateTransformOption("strip_metering", checked === true)
              }
            >
              Remove usage and metering details
            </Checkbox>
          </div>

          {transformPreview && (
            <Box className={styles.previewSection}>
              <Text size="2" weight="medium">
                ~
                {transformPreview.stats.before_approx_tokens > 0
                  ? Math.round(
                      ((transformPreview.stats.before_approx_tokens -
                        transformPreview.stats.after_approx_tokens) /
                        transformPreview.stats.before_approx_tokens) *
                        100,
                    )
                  : 0}
                % reduction (approximate)
              </Text>
              {transformPreview.actions.length > 0 && (
                <ul className={styles.actionsList}>
                  {transformPreview.actions.map((action, idx) => (
                    <li key={idx} className={styles.actionsListItem}>
                      {action}
                    </li>
                  ))}
                </ul>
              )}
            </Box>
          )}

          <Flex className={styles.buttonRow}>
            <Button
              variant="soft"
              size="sm"
              loading={isPreviewingTransform}
              onClick={() => {
                void handlePreviewTransform();
              }}
            >
              Preview
            </Button>
            <Button
              size="sm"
              loading={isApplyingTransform}
              onClick={() => {
                void handleApplyTransformClick();
              }}
              disabled={!transformPreview}
            >
              Apply
            </Button>
          </Flex>
        </Tabs.Content>

        <Tabs.Content value="llm-compress">
          <div className={styles.llmIntro}>
            <Text size="2">
              Rebuilds the active conversation context with an LLM. Original
              chat messages remain visible in the transcript. The selected
              provider receives only content allowed by your privacy policy.
            </Text>
          </div>

          <div className={styles.modelSection}>
            <Text size="2" weight="medium">
              Summary model
            </Text>
            {caps.usableModels.length > 0 ? (
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
            ) : (
              <Text size="2" color="gray">
                Configure an eligible chat model in Providers before using LLM
                compression.
              </Text>
            )}
            {!llmCompressOptions.summary_model && caps.currentModel && (
              <Text size="1" color="gray">
                Current chat model: {caps.currentModel}
              </Text>
            )}
          </div>

          {llmCompressPreview && (
            <Box className={styles.previewSection}>
              <Text size="2" weight="medium">
                {llmCompressPreview.eligible
                  ? `${
                      llmCompressPreview.source_messages
                    } messages (~${llmCompressPreview.approximate_source_tokens.toLocaleString()} tokens) eligible`
                  : "Context rebuild is not currently available"}
              </Text>
              <div className={styles.previewDetails}>
                {llmCompressPreview.resolved_model && (
                  <Text size="1" color="gray">
                    Model: {llmCompressPreview.resolved_model}
                  </Text>
                )}
                {llmCompressPreview.context_window !== undefined && (
                  <Text size="1" color="gray">
                    Context window:{" "}
                    {formatContextWindow(llmCompressPreview.context_window)}
                  </Text>
                )}
              </div>
              {llmCompressPreview.reason && (
                <Text size="2" color="red">
                  {llmCompressPreview.reason}
                </Text>
              )}
            </Box>
          )}

          {llmCompressError && (
            <Box className={styles.errorCallout} role="alert">
              <Text size="2">{llmCompressError}</Text>
            </Box>
          )}

          <Flex className={styles.buttonRow}>
            <Button
              variant="soft"
              size="sm"
              loading={isPreviewingLlmCompress}
              disabled={caps.usableModels.length === 0}
              onClick={() => {
                void handlePreviewLlmCompress();
              }}
            >
              Preview
            </Button>
            <Button
              size="sm"
              loading={isApplyingLlmCompress}
              disabled={!llmCompressPreview?.eligible}
              onClick={() => {
                void handleApplyLlmCompressClick();
              }}
            >
              Rebuild context
            </Button>
          </Flex>
        </Tabs.Content>
      </Tabs>
    </Popover.Content>
  );
};
