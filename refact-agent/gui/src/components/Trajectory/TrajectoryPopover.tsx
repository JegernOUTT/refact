import React from "react";
import { Box, Flex, Popover, Text } from "../LongTailPrimitives";
import { Checkbox } from "../Checkbox";
import { Button, Tabs } from "../ui";
import { useTrajectoryOps } from "../../hooks/useTrajectoryOps";
import styles from "./TrajectoryPopover.module.css";

const TAB_OPTIONS = [
  { value: "compress", label: "Compress in-place" },
  { value: "handoff", label: "Handoff" },
];

type TrajectoryPopoverContentProps = {
  onClose: () => void;
};

export const TrajectoryPopoverContent: React.FC<
  TrajectoryPopoverContentProps
> = ({ onClose }) => {
  const {
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
  } = useTrajectoryOps();

  const handleTabChange = (value: string) => {
    setActiveTab(value as "compress" | "handoff");
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

  const handleApplyHandoffClick = async () => {
    const success = await handleApplyHandoff();
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
      maxWidth="min(360px, calc(100vw - var(--rf-space-4)))"
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

        <Tabs.Content value="handoff">
          <div className={styles.optionsSection}>
            <Checkbox
              checked={handoffOptions.include_last_user_plus}
              onCheckedChange={(checked) =>
                updateHandoffOption("include_last_user_plus", checked === true)
              }
            >
              Include last user message + responses
            </Checkbox>
            <Checkbox
              checked={handoffOptions.include_all_opened_context}
              onCheckedChange={(checked) =>
                updateHandoffOption(
                  "include_all_opened_context",
                  checked === true,
                )
              }
            >
              Include all opened files
            </Checkbox>
            <Checkbox
              checked={handoffOptions.include_agentic_tools}
              onCheckedChange={(checked) =>
                updateHandoffOption("include_agentic_tools", checked === true)
              }
            >
              Include research, subagent & planning results
            </Checkbox>
            <Checkbox
              checked={handoffOptions.llm_summary_for_excluded}
              onCheckedChange={(checked) =>
                updateHandoffOption(
                  "llm_summary_for_excluded",
                  checked === true,
                )
              }
            >
              Generate summary
            </Checkbox>
            <Checkbox
              checked={handoffOptions.include_all_user_assistant_only}
              onCheckedChange={(checked) =>
                updateHandoffOption(
                  "include_all_user_assistant_only",
                  checked === true,
                )
              }
            >
              Include all user messages + responses
            </Checkbox>
          </div>

          {handoffPreview && (
            <Box className={styles.previewSection}>
              <Text size="2" weight="medium" mb="2">
                ~
                {handoffPreview.stats.before_approx_tokens > 0
                  ? Math.round(
                      ((handoffPreview.stats.before_approx_tokens -
                        handoffPreview.stats.after_approx_tokens) /
                        handoffPreview.stats.before_approx_tokens) *
                        100,
                    )
                  : 0}
                % reduction (approximate)
              </Text>
              {handoffPreview.actions.length > 0 && (
                <ul className={styles.actionsList}>
                  {handoffPreview.actions.map((action, idx) => (
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
              loading={isPreviewingHandoff}
              onClick={() => {
                void handlePreviewHandoff();
              }}
            >
              Preview
            </Button>
            <Button
              size="sm"
              loading={isApplyingHandoff}
              onClick={() => {
                void handleApplyHandoffClick();
              }}
              disabled={!handoffPreview}
            >
              Create
            </Button>
          </Flex>
        </Tabs.Content>
      </Tabs>
    </Popover.Content>
  );
};
