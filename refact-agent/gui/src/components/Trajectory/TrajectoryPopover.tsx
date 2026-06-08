import React from "react";
import {
  Box,
  Button,
  Checkbox,
  Flex,
  Popover,
  Spinner,
  Text,
} from "../LongTailPrimitives";
import { SegmentedControl } from "../ui";
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
    >
      <SegmentedControl
        className={styles.tabStrip}
        size="sm"
        options={TAB_OPTIONS}
        value={activeTab}
        onValueChange={handleTabChange}
      />

      {activeTab === "compress" && (
        <>
          <div className={styles.optionsSection}>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={transformOptions.drop_all_context}
                  onCheckedChange={(checked) => {
                    const enabled = checked;
                    updateTransformOption("drop_all_context", enabled);
                    if (enabled) {
                      updateTransformOption(
                        "dedup_and_compress_context",
                        false,
                      );
                    }
                  }}
                />
                Drop all context files
              </Flex>
            </Text>
            <Text
              as="label"
              size="2"
              color={transformOptions.drop_all_context ? "gray" : undefined}
              style={{ marginLeft: "24px" }}
            >
              <Flex gap="2" align="center">
                <Checkbox
                  checked={transformOptions.dedup_and_compress_context}
                  disabled={transformOptions.drop_all_context}
                  onCheckedChange={(checked) =>
                    updateTransformOption("dedup_and_compress_context", checked)
                  }
                />
                Deduplicate context files
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={transformOptions.compress_non_agentic_tools}
                  onCheckedChange={(checked) =>
                    updateTransformOption("compress_non_agentic_tools", checked)
                  }
                />
                Truncate tool results
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={transformOptions.drop_all_memories}
                  onCheckedChange={(checked) =>
                    updateTransformOption("drop_all_memories", checked)
                  }
                />
                Drop all memories
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={transformOptions.drop_project_information}
                  onCheckedChange={(checked) =>
                    updateTransformOption("drop_project_information", checked)
                  }
                />
                Drop project information
              </Flex>
            </Text>
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
              onClick={() => {
                void handlePreviewTransform();
              }}
              disabled={isPreviewingTransform}
            >
              {isPreviewingTransform ? <Spinner size="1" /> : "Preview"}
            </Button>
            <Button
              onClick={() => {
                void handleApplyTransformClick();
              }}
              disabled={!transformPreview || isApplyingTransform}
            >
              {isApplyingTransform ? <Spinner size="1" /> : "Apply"}
            </Button>
          </Flex>
        </>
      )}

      {activeTab === "handoff" && (
        <>
          <div className={styles.optionsSection}>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={handoffOptions.include_last_user_plus}
                  onCheckedChange={(checked) =>
                    updateHandoffOption("include_last_user_plus", checked)
                  }
                />
                Include last user message + responses
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={handoffOptions.include_all_opened_context}
                  onCheckedChange={(checked) =>
                    updateHandoffOption("include_all_opened_context", checked)
                  }
                />
                Include all opened files
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={handoffOptions.include_agentic_tools}
                  onCheckedChange={(checked) =>
                    updateHandoffOption("include_agentic_tools", checked)
                  }
                />
                Include research, subagent & planning results
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={handoffOptions.llm_summary_for_excluded}
                  onCheckedChange={(checked) =>
                    updateHandoffOption("llm_summary_for_excluded", checked)
                  }
                />
                Generate summary
              </Flex>
            </Text>
            <Text as="label" size="2">
              <Flex gap="2" align="center">
                <Checkbox
                  checked={handoffOptions.include_all_user_assistant_only}
                  onCheckedChange={(checked) =>
                    updateHandoffOption(
                      "include_all_user_assistant_only",
                      checked,
                    )
                  }
                />
                Include all user messages + responses
              </Flex>
            </Text>
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
              onClick={() => {
                void handlePreviewHandoff();
              }}
              disabled={isPreviewingHandoff}
            >
              {isPreviewingHandoff ? <Spinner size="1" /> : "Preview"}
            </Button>
            <Button
              onClick={() => {
                void handleApplyHandoffClick();
              }}
              disabled={!handoffPreview || isApplyingHandoff}
            >
              {isApplyingHandoff ? <Spinner size="1" /> : "Create"}
            </Button>
          </Flex>
        </>
      )}
    </Popover.Content>
  );
};
