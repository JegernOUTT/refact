import React from "react";
import {
  Box,
  Button,
  Checkbox,
  Flex,
  Popover,
  Spinner,
  Tabs,
  Text,
} from "@radix-ui/themes";
import { useTrajectoryOps } from "../../hooks/useTrajectoryOps";
import styles from "./TrajectoryPopover.module.css";

type TrajectoryPopoverContentProps = {
  onClose: () => void;
};

export const TrajectoryPopoverContent: React.FC<TrajectoryPopoverContentProps> = ({
  onClose,
}) => {
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
        side="top"
        align="end"
        sideOffset={8}
        className={styles.popoverContent}
      >
        <Tabs.Root value={activeTab} onValueChange={handleTabChange}>
          <Tabs.List className={styles.tabsList}>
            <Tabs.Trigger value="compress" className={styles.tabsTrigger}>
              Compress
            </Tabs.Trigger>
            <Tabs.Trigger value="handoff" className={styles.tabsTrigger}>
              Handoff
            </Tabs.Trigger>
          </Tabs.List>

          <Tabs.Content value="compress">
            <Box className={styles.optionsSection}>
              <Text as="label" size="2">
                <Flex gap="2" align="center">
                  <Checkbox
                    checked={transformOptions.dedup_and_compress_context}
                    onCheckedChange={(checked) =>
                      updateTransformOption("dedup_and_compress_context", checked === true)
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
                      updateTransformOption("compress_non_agentic_tools", checked === true)
                    }
                  />
                  Compress tool results
                </Flex>
              </Text>
              <Text as="label" size="2">
                <Flex gap="2" align="center">
                  <Checkbox
                    checked={transformOptions.drop_all_context}
                    onCheckedChange={(checked) =>
                      updateTransformOption("drop_all_context", checked === true)
                    }
                  />
                  Drop all context files
                </Flex>
              </Text>
              <Text as="label" size="2">
                <Flex gap="2" align="center">
                  <Checkbox
                    checked={transformOptions.drop_all_memories}
                    onCheckedChange={(checked) =>
                      updateTransformOption("drop_all_memories", checked === true)
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
                      updateTransformOption("drop_project_information", checked === true)
                    }
                  />
                  Drop project information
                </Flex>
              </Text>
            </Box>

            {transformPreview && (
              <Box className={styles.previewSection}>
                <Flex className={styles.previewStats}>
                  <Text size="1" color="gray">
                    Before: {transformPreview.stats.before_approx_tokens} tokens
                  </Text>
                  <Text size="1" color="gray">
                    After: {transformPreview.stats.after_approx_tokens} tokens
                  </Text>
                </Flex>
                <Text size="2" weight="medium">
                  ~{transformPreview.stats.before_approx_tokens > 0
                    ? Math.round(((transformPreview.stats.before_approx_tokens - transformPreview.stats.after_approx_tokens) / transformPreview.stats.before_approx_tokens) * 100)
                    : 0}% reduction
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
                onClick={handlePreviewTransform}
                disabled={isPreviewingTransform}
              >
                {isPreviewingTransform ? <Spinner size="1" /> : "Preview"}
              </Button>
              <Button
                onClick={handleApplyTransformClick}
                disabled={!transformPreview || isApplyingTransform}
              >
                {isApplyingTransform ? <Spinner size="1" /> : "Apply"}
              </Button>
            </Flex>
          </Tabs.Content>

          <Tabs.Content value="handoff">
            <Box className={styles.optionsSection}>
              <Text as="label" size="2">
                <Flex gap="2" align="center">
                  <Checkbox
                    checked={handoffOptions.include_last_user_plus}
                    onCheckedChange={(checked) =>
                      updateHandoffOption("include_last_user_plus", checked === true)
                    }
                  />
                  From last user message only
                </Flex>
              </Text>
              <Text as="label" size="2">
                <Flex gap="2" align="center">
                  <Checkbox
                    checked={handoffOptions.include_all_opened_context}
                    onCheckedChange={(checked) =>
                      updateHandoffOption("include_all_opened_context", checked === true)
                    }
                  />
                  Include context files
                </Flex>
              </Text>
              <Text as="label" size="2">
                <Flex gap="2" align="center">
                  <Checkbox
                    checked={handoffOptions.include_agentic_tools}
                    onCheckedChange={(checked) =>
                      updateHandoffOption("include_agentic_tools", checked === true)
                    }
                  />
                  Include tool results
                </Flex>
              </Text>
              <Text as="label" size="2">
                <Flex gap="2" align="center">
                  <Checkbox
                    checked={handoffOptions.llm_summary_for_excluded}
                    onCheckedChange={(checked) =>
                      updateHandoffOption("llm_summary_for_excluded", checked === true)
                    }
                  />
                  Generate summary
                </Flex>
              </Text>
            </Box>

            {handoffPreview && (
              <Box className={styles.previewSection}>
                <Text size="1" color="gray" mb="2">
                  ~{handoffPreview.stats.after_approx_tokens} tokens
                  {handoffOptions.llm_summary_for_excluded && " (inaccurate, summary will be generated on Create)"}
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
                onClick={handlePreviewHandoff}
                disabled={isPreviewingHandoff}
              >
                {isPreviewingHandoff ? <Spinner size="1" /> : "Preview"}
              </Button>
              <Button
                onClick={handleApplyHandoffClick}
                disabled={!handoffPreview || isApplyingHandoff}
              >
                {isApplyingHandoff ? <Spinner size="1" /> : "Create"}
              </Button>
            </Flex>
          </Tabs.Content>
        </Tabs.Root>
      </Popover.Content>
  );
};
