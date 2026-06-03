import React from "react";
import { Flex, HoverCard, Skeleton, Text } from "@radix-ui/themes";
import { ChevronDownIcon, ChevronUpIcon } from "@radix-ui/react-icons";
import { CollapsePanel } from "../../../../components/shared/CollapsePanel";
import { useGetCapsQuery } from "../../../../services/refact/caps";
import { useAppDispatch } from "../../../../hooks";
import { push } from "../../../Pages/pagesSlice";
import type { DashboardBreakpoint } from "../../types";
import styles from "./ModelsSection.module.css";

type ModelsSectionProps = {
  breakpoint: DashboardBreakpoint;
  collapsed: boolean;
  onToggleCollapsed: () => void;
};

function ModelRow({
  label,
  model,
  explanation,
}: {
  label: string;
  model: string;
  explanation: string;
}) {
  const shortName = model.split("/").pop() ?? model;
  return (
    <HoverCard.Root openDelay={300} closeDelay={100}>
      <HoverCard.Trigger>
        <div className={styles.modelRow}>
          <Text size="1" color="gray" className={styles.modelLabel}>
            {label}
          </Text>
          <Text size="1" weight="medium" truncate className={styles.modelName}>
            {shortName}
          </Text>
        </div>
      </HoverCard.Trigger>
      <HoverCard.Content
        size="1"
        side="top"
        align="center"
        className={styles.hoverCard}
        avoidCollisions
      >
        <Flex direction="column" gap="1">
          <Text size="2" weight="bold">
            {label}
          </Text>
          <Text size="1" color="gray">
            {explanation}
          </Text>
          <Text size="1">Current: {model}</Text>
        </Flex>
      </HoverCard.Content>
    </HoverCard.Root>
  );
}

export const ModelsSection: React.FC<ModelsSectionProps> = ({
  breakpoint,
  collapsed,
  onToggleCollapsed,
}) => {
  const dispatch = useAppDispatch();
  const { data: caps, isLoading, isError } = useGetCapsQuery(undefined);

  if (isLoading) {
    return (
      <div className={styles.section}>
        <div className={styles.header}>
          <button
            type="button"
            className={styles.headerToggle}
            onClick={onToggleCollapsed}
            aria-expanded={!collapsed}
          >
            <Text size="1" weight="bold" color="gray" className={styles.label}>
              DEFAULT MODELS
            </Text>
            {collapsed ? (
              <ChevronDownIcon width={12} height={12} color="var(--gray-9)" />
            ) : (
              <ChevronUpIcon width={12} height={12} color="var(--gray-9)" />
            )}
          </button>
        </div>
        <CollapsePanel collapsed={collapsed}>
          <Skeleton height="24px" />
        </CollapsePanel>
      </div>
    );
  }

  if (isError || !caps) return null;

  const chatModelCount = Object.keys(caps.chat_models).length;
  const completionModelCount = Object.keys(caps.completion_models).length;

  return (
    <div className={styles.section}>
      <div className={styles.header}>
        <button
          type="button"
          className={styles.headerToggle}
          onClick={onToggleCollapsed}
          aria-expanded={!collapsed}
        >
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            DEFAULT MODELS
          </Text>
          {collapsed ? (
            <ChevronDownIcon width={12} height={12} color="var(--gray-9)" />
          ) : (
            <ChevronUpIcon width={12} height={12} color="var(--gray-9)" />
          )}
        </button>
        <button
          type="button"
          className={styles.configureButton}
          onClick={() => dispatch(push({ name: "default models" }))}
        >
          <Text size="1">Configure</Text>
        </button>
      </div>
      <CollapsePanel collapsed={collapsed}>
        <div className={styles.models} data-breakpoint={breakpoint}>
          {caps.chat_default_model && (
            <ModelRow
              label="Chat"
              model={caps.chat_default_model}
              explanation="Primary model for chat conversations. Used for most interactions."
            />
          )}
          {caps.chat_model_2 &&
            caps.chat_model_2 !== caps.chat_default_model && (
              <ModelRow
                label="Chat 2"
                model={caps.chat_model_2}
                explanation="Secondary chat model slot for future chat workflows."
              />
            )}
          {caps.task_planner_agent_model &&
            caps.task_planner_agent_model !== caps.chat_default_model &&
            caps.task_planner_agent_model !== caps.chat_model_2 && (
              <ModelRow
                label="Task Agent"
                model={caps.task_planner_agent_model}
                explanation="Model used by task management when spawning task agents."
              />
            )}
          {caps.chat_thinking_model &&
            caps.chat_thinking_model !== caps.chat_default_model && (
              <ModelRow
                label="Thinking"
                model={caps.chat_thinking_model}
                explanation="Model with extended reasoning for complex tasks. Uses thinking/reasoning tokens for step-by-step problem solving."
              />
            )}
          {caps.chat_light_model &&
            caps.chat_light_model !== caps.chat_default_model && (
              <ModelRow
                label="Light"
                model={caps.chat_light_model}
                explanation="Faster, cheaper model for simple tasks like title generation, quick lookups, and subagent calls."
              />
            )}
          {caps.chat_buddy_model &&
            caps.chat_buddy_model !== caps.chat_default_model &&
            caps.chat_buddy_model !== caps.chat_light_model && (
              <ModelRow
                label="Companion"
                model={caps.chat_buddy_model}
                explanation="Model used by your companion for background tasks and suggestions."
              />
            )}
          {caps.completion_default_model && (
            <ModelRow
              label="Completion"
              model={caps.completion_default_model}
              explanation="Model for inline code completion (autocomplete). Optimized for fill-in-the-middle predictions."
            />
          )}
        </div>
        <Text size="1" color="gray" className={styles.modelCount}>
          {chatModelCount} chat + {completionModelCount} completion models
          available
        </Text>
      </CollapsePanel>
    </div>
  );
};
