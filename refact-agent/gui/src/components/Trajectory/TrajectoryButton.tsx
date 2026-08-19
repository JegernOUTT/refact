import React, { useState } from "react";
import { HoverCard, Popover, Text } from "../LongTailPrimitives";
import { Archive } from "lucide-react";
import { IconButton } from "../ui";
import { TrajectoryPopoverContent } from "./TrajectoryPopover";

type TrajectoryButtonProps = {
  forceOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  disabled?: boolean;
};

export const TrajectoryButton: React.FC<TrajectoryButtonProps> = ({
  forceOpen,
  onOpenChange,
  disabled,
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

  return (
    <Popover.Root open={open} onOpenChange={handleOpenChange}>
      <HoverCard.Root openDelay={300}>
        <HoverCard.Trigger asChild>
          <Popover.Trigger asChild>
            <IconButton
              data-testid="trajectory-button"
              aria-label="Compress or Handoff"
              disabled={disabled}
              icon={Archive}
              size="sm"
              variant="ghost"
            />
          </Popover.Trigger>
        </HoverCard.Trigger>
        <HoverCard.Content size="1" side="bottom">
          <Text as="p" size="2">
            Compress or Handoff
          </Text>
        </HoverCard.Content>
      </HoverCard.Root>
      <TrajectoryPopoverContent onClose={() => handleOpenChange(false)} />
    </Popover.Root>
  );
};
