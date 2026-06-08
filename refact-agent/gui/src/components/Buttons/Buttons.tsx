import React, { forwardRef } from "react";
import { Flex, HoverCard, Text } from "@radix-ui/themes";
import { FileText, LogOut, Puzzle, Send, X } from "lucide-react";
import classNames from "classnames";
import { Button, ButtonGroup, IconButton } from "../ui";
import styles from "./button.module.css";

type KitIconButtonProps = Omit<
  React.ComponentProps<typeof IconButton>,
  "icon" | "aria-label"
> & {
  "aria-label"?: string;
};
type KitButtonProps = React.ComponentProps<typeof Button>;
type PlainButtonProps = React.ButtonHTMLAttributes<HTMLButtonElement>;

type LegacyButtonSize =
  | React.ComponentProps<typeof IconButton>["size"]
  | "1"
  | "2"
  | "3";

function normalizeSize(
  size: LegacyButtonSize | undefined,
): React.ComponentProps<typeof IconButton>["size"] {
  if (size === "1") return "sm";
  if (size === "2") return "md";
  if (size === "3") return "lg";
  return size ?? "md";
}

export const PaperPlaneButton: React.FC<KitIconButtonProps> = ({
  "aria-label": ariaLabel = "Send message",
  ...props
}) => (
  <IconButton aria-label={ariaLabel} icon={Send} variant="ghost" {...props} />
);

export const AgentIntegrationsButton = forwardRef<
  HTMLButtonElement,
  PlainButtonProps
>((props, ref) => (
  <HoverCard.Root>
    <HoverCard.Trigger>
      <IconButton
        {...props}
        ref={ref}
        aria-label="Set up Agent Integrations"
        icon={Puzzle}
        size="sm"
        variant="ghost"
      />
    </HoverCard.Trigger>
    <HoverCard.Content size="1" side="top">
      <Text as="p" size="2">
        Set up Agent Integrations
      </Text>
    </HoverCard.Content>
  </HoverCard.Root>
));

AgentIntegrationsButton.displayName = "AgentIntegrationsButton";

export const ThreadHistoryButton: React.FC<KitIconButtonProps> = ({
  "aria-label": ariaLabel = "Thread history",
  ...props
}) => (
  <IconButton
    aria-label={ariaLabel}
    icon={FileText}
    variant="ghost"
    {...props}
  />
);

export function BackToSideBarButton(props: PlainButtonProps) {
  return (
    <HoverCard.Root>
      <HoverCard.Trigger>
        <IconButton
          {...props}
          aria-label="Return to sidebar"
          className={styles.flipIcon}
          icon={LogOut}
          size="sm"
          variant="ghost"
        />
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="top">
        <Text as="p" size="2">
          Return to sidebar
        </Text>
      </HoverCard.Content>
    </HoverCard.Root>
  );
}

export const CloseButton: React.FC<
  Omit<KitIconButtonProps, "size"> & {
    iconSize?: number | string;
    size?: LegacyButtonSize;
  }
> = ({
  "aria-label": ariaLabel = "Close",
  iconSize: _iconSize,
  size,
  ...props
}) => (
  <IconButton
    aria-label={ariaLabel}
    icon={X}
    size={normalizeSize(size)}
    variant="ghost"
    {...props}
  />
);

export const RightButton: React.FC<KitButtonProps & { className?: string }> = (
  props,
) => {
  return (
    <Button
      size="sm"
      variant="soft"
      {...props}
      className={classNames(styles.rightButton, props.className)}
    />
  );
};

type FlexProps = React.ComponentProps<typeof Flex>;

export const RightButtonGroup: React.FC<React.PropsWithChildren & FlexProps> = (
  props,
) => {
  return (
    <Flex
      {...props}
      gap="1"
      className={classNames(styles.rightButtonGroup, props.className)}
    />
  );
};

export { ButtonGroup };
