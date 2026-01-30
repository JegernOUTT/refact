import React, { useState } from "react";
import { HoverCard, IconButton, Popover, Text } from "@radix-ui/themes";
import { ArchiveIcon } from "@radix-ui/react-icons";
import { TrajectoryPopoverContent } from "./TrajectoryPopover";

type TrajectoryButtonProps = {
  forceOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export const TrajectoryButton: React.FC<TrajectoryButtonProps> = ({
  forceOpen,
  onOpenChange,
}) => {
  const [internalOpen, setInternalOpen] = useState(false);
  const isControlled = forceOpen !== undefined;
  const open = isControlled ? forceOpen : internalOpen;

  const handleOpenChange = (newOpen: boolean) => {
    if (!isControlled) {
      setInternalOpen(newOpen);
    }
    onOpenChange?.(newOpen);
  };

  return (
    <Popover.Root open={open} onOpenChange={handleOpenChange}>
      <HoverCard.Root openDelay={300}>
        <HoverCard.Trigger>
          <Popover.Trigger>
            <IconButton
              type="button"
              variant="ghost"
              size="1"
              data-testid="trajectory-button"
              aria-label="Open trajectory options"
            >
              <ArchiveIcon />
            </IconButton>
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
