import type { FC } from "react";
import { Flex } from "@radix-ui/themes";
import {
  ChatBubbleIcon,
  ImageIcon,
  CursorArrowIcon,
  RocketIcon,
  GearIcon,
} from "@radix-ui/react-icons";
import type { ModelCapabilities } from "../utils/groupModelsWithPricing";

export type CapabilityIconsProps = {
  capabilities?: ModelCapabilities;
  size?: "1" | "2";
};

export const CapabilityIcons: FC<CapabilityIconsProps> = ({
  capabilities,
  size = "1",
}) => {
  if (!capabilities) return null;

  const iconSize = size === "1" ? 12 : 14;
  const iconStyle = { width: iconSize, height: iconSize };

  return (
    <Flex gap="1" align="center">
      {capabilities.supportsTools && (
        <span title="Supports tools">
          <GearIcon style={iconStyle} color="var(--gray-11)" />
        </span>
      )}
      {capabilities.supportsMultimodality && (
        <span title="Supports images">
          <ImageIcon style={iconStyle} color="var(--gray-11)" />
        </span>
      )}
      {capabilities.supportsClicks && (
        <span title="Computer use">
          <CursorArrowIcon style={iconStyle} color="var(--gray-11)" />
        </span>
      )}
      {capabilities.supportsAgent && (
        <span title="Agent mode">
          <RocketIcon style={iconStyle} color="var(--gray-11)" />
        </span>
      )}
      {(!!capabilities.reasoningEffortOptions?.length ||
        !!capabilities.supportsThinkingBudget ||
        !!capabilities.supportsAdaptiveThinkingBudget) && (
        <span title="Reasoning">
          <ChatBubbleIcon style={iconStyle} color="var(--blue-11)" />
        </span>
      )}
    </Flex>
  );
};
