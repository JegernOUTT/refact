import type { ComponentProps, FC, ReactNode } from "react";
import { Flex, Text } from "@radix-ui/themes";
import {
  ChatBubbleIcon,
  ImageIcon,
  CursorArrowIcon,
  RocketIcon,
  GearIcon,
  BarChartIcon,
  EyeOpenIcon,
  WidthIcon,
  ScissorsIcon,
} from "@radix-ui/react-icons";
import type { ModelCapabilities } from "../utils/groupModelsWithPricing";
import styles from "../ModelCard.module.css";

export type CapabilityIconsProps = {
  capabilities?: ModelCapabilities;
  size?: "1" | "2";
};

type ModelDetailIconProps = {
  icon: ReactNode;
  children?: ReactNode;
  color?: ComponentProps<typeof Text>["color"];
  tone?: "default" | "accent";
};

export const ModelDetailIcon: FC<ModelDetailIconProps> = ({
  icon,
  children,
  color = "gray",
  tone = "default",
}) => (
  <Text as="span" size="1" color={color}>
    <Flex as="span" align="center" gap="1" className={styles.modelDetailIcon}>
      <span
        className={
          tone === "accent"
            ? styles.modelDetailIconGlyphAccent
            : styles.modelDetailIconGlyph
        }
      >
        {icon}
      </span>
      {children}
    </Flex>
  </Text>
);

type DetailSvgIconProps = ComponentProps<typeof WidthIcon>;

export const ContextWindowIcon: FC<DetailSvgIconProps> = (props) => (
  <WidthIcon {...props} />
);
export const MaxOutputIcon: FC<DetailSvgIconProps> = (props) => (
  <ScissorsIcon {...props} />
);
export const PricingIcon: FC<DetailSvgIconProps> = (props) => (
  <BarChartIcon {...props} />
);
export const ToolsIcon: FC<DetailSvgIconProps> = (props) => (
  <GearIcon {...props} />
);
export const VisionIcon: FC<DetailSvgIconProps> = (props) => (
  <EyeOpenIcon {...props} />
);
export const ReasoningIcon: FC<DetailSvgIconProps> = (props) => (
  <ChatBubbleIcon {...props} />
);

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
          <ToolsIcon style={iconStyle} color="var(--gray-11)" />
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
