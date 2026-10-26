import React, { useState } from "react";
import { HoverCard, Popover, Text } from "../LongTailPrimitives";
import { Archive } from "lucide-react";
import { Button, IconButton } from "../ui";
import { TrajectoryPopoverContent } from "./TrajectoryPopover";
import type { TrajectoryTab } from "../../hooks/useTrajectoryOps";

type TrajectoryButtonProps = {
  forceOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  disabled?: boolean;
  label?: string;
  initialTab?: TrajectoryTab;
  chatId?: string;
};

export const TrajectoryButton: React.FC<TrajectoryButtonProps> = ({
  forceOpen,
  onOpenChange,
  disabled,
  label,
  initialTab,
  chatId,
}) => {
  const [internalOpen, setInternalOpen] = useState(false);
  const isControlled = forceOpen !== undefined;
  const open = isControlled ? forceOpen : internalOpen;

  const handleOpenChange = (newOpen: boolean) => {
    if (disabled && newOpen) return;
    if (!isControlled) {
      setInternalOpen(newOpen);
    }
    onOpenChange?.(newOpen);
  };

  const content = (
    <TrajectoryPopoverContent
      initialTab={initialTab}
      chatId={chatId}
      onClose={() => handleOpenChange(false)}
    />
  );

  if (label) {
    return (
      <Popover.Root open={open} onOpenChange={handleOpenChange}>
        <Popover.Trigger asChild>
          <Button
            disabled={disabled}
            leftIcon={Archive}
            size="sm"
            variant="soft"
          >
            {label}
          </Button>
        </Popover.Trigger>
        {content}
      </Popover.Root>
    );
  }

  return (
    <Popover.Root open={open} onOpenChange={handleOpenChange}>
      <HoverCard.Root openDelay={300}>
        <HoverCard.Trigger asChild>
          <Popover.Trigger asChild>
            <IconButton
              data-testid="trajectory-button"
              aria-label="Compress or rebuild context"
              disabled={disabled}
              icon={Archive}
              size="sm"
              variant="ghost"
            />
          </Popover.Trigger>
        </HoverCard.Trigger>
        <HoverCard.Content size="1" side="bottom">
          <Text as="p" size="2">
            Compress or rebuild context
          </Text>
        </HoverCard.Content>
      </HoverCard.Root>
      {content}
    </Popover.Root>
  );
};
