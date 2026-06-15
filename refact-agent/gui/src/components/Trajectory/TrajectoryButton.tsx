import React, { useState } from "react";
import { HoverCard, Popover, Text } from "../LongTailPrimitives";
import { ArchiveIcon } from "@radix-ui/react-icons";
import { TrajectoryPopoverContent } from "./TrajectoryPopover";
import styles from "./TrajectoryButton.module.css";

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
            <button
              type="button"
              className={styles.iconButton}
              data-testid="trajectory-button"
              aria-label="Compress or Handoff"
              disabled={disabled}
            >
              <ArchiveIcon />
            </button>
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
