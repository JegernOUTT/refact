import React, { useState } from "react";
import { IconButton, Popover } from "@radix-ui/themes";
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
      <Popover.Trigger>
        <IconButton
          variant="ghost"
          size="1"
          title="Trajectory: Compress or Handoff"
          data-testid="trajectory-button"
          aria-label="Open trajectory options"
        >
          <ArchiveIcon />
        </IconButton>
      </Popover.Trigger>
      <TrajectoryPopoverContent onClose={() => handleOpenChange(false)} />
    </Popover.Root>
  );
};
