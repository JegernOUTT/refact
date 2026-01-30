import React, { useRef, useEffect, useState } from "react";
import { Flex, Text, Badge, Skeleton, Popover, Separator } from "@radix-ui/themes";
import {
  useGetChatModesQuery,
  ChatModeInfo,
} from "../../services/refact/chatModes";
import { DEFAULT_MODE } from "../../features/Chat/Thread/types";
import { useAppSelector, useAppDispatch } from "../../hooks";
import { selectMessages } from "../../features/Chat/Thread";
import { push } from "../../features/Pages/pagesSlice";
import styles from "./ModeSelect.module.css";

type ModeSelectProps = {
  selectedMode: string;
  onModeChange: (modeId: string) => void;
  disabled?: boolean;
};

export const ModeSelect: React.FC<ModeSelectProps> = ({
  selectedMode,
  onModeChange,
  disabled,
}) => {
  const dispatch = useAppDispatch();
  const { data, isLoading, isError } = useGetChatModesQuery(undefined);
  const messages = useAppSelector(selectMessages);

  const modes = data?.modes ?? [];
  const effectiveMode = selectedMode || DEFAULT_MODE;
  const currentMode = modes.find((m) => m.id === effectiveMode);
  const currentTitle = currentMode?.title ?? effectiveMode;
  const toolsCount = currentMode?.tools_count ?? 0;

  // Mode is locked after first message
  const isModeLocked = messages.length > 0;
  const isModeDisabled = disabled ?? isModeLocked;

  const [isOpen, setIsOpen] = useState(false);
  const selectedModeRef = useRef<HTMLButtonElement>(null);
  const modeListRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isOpen) return;

    const scrollToSelected = () => {
      const container = modeListRef.current;
      const selected = selectedModeRef.current;
      if (container && selected && container.clientHeight > 0) {
        const containerHeight = container.clientHeight;
        const selectedTop = selected.offsetTop;
        const selectedHeight = selected.offsetHeight;
        container.scrollTop = selectedTop - (containerHeight / 2) + (selectedHeight / 2);
        return true;
      }
      return false;
    };

    let attempts = 0;
    const maxAttempts = 10;
    const tryScroll = () => {
      if (scrollToSelected() || attempts >= maxAttempts) return;
      attempts++;
      requestAnimationFrame(tryScroll);
    };

    requestAnimationFrame(tryScroll);
  }, [isOpen]);

  const handleCreateNewMode = () => {
    dispatch(push({ name: "customization", kind: "modes" }));
    setIsOpen(false);
  };

  if (isLoading) {
    return (
      <Skeleton>
        <div className={styles.trigger}>
          <Text size="1">Loading...</Text>
        </div>
      </Skeleton>
    );
  }

  if (isError || modes.length === 0) {
    return (
      <div className={`${styles.trigger} ${styles.disabled}`}>
        <Text size="1" color="gray">
          {isError ? "Error" : "No modes"}
        </Text>
      </div>
    );
  }

  const triggerContent = (
    <Flex align="center" gap="1" className={styles.triggerContent}>
      <Text size="1">{currentTitle}</Text>
      {toolsCount > 0 && (
        <>
          <Text size="1" color="gray">·</Text>
          <Text size="1" color="gray">{toolsCount} tools</Text>
        </>
      )}
    </Flex>
  );

  return (
    <Popover.Root open={isOpen} onOpenChange={setIsOpen}>
      <Popover.Trigger>
        <button
          className={`${styles.trigger} ${isModeDisabled ? styles.disabled : ""}`}
          disabled={isModeDisabled}
          type="button"
          title={isModeLocked ? "Mode is locked after first message" : undefined}
        >
          {triggerContent}
        </button>
      </Popover.Trigger>

      <Popover.Content
        className={styles.content}
        side="top"
        align="start"
        sideOffset={8}
      >
        <div className={styles.modeList} ref={modeListRef}>
          {modes.map((mode, index) => {
            const isSelected = effectiveMode === mode.id;
            return (
              <React.Fragment key={mode.id}>
                {index > 0 && <Separator size="4" className={styles.separator} />}
                <ModeMenuItem
                  ref={isSelected ? selectedModeRef : undefined}
                  mode={mode}
                  isSelected={isSelected}
                  onSelect={() => {
                    onModeChange(mode.id);
                    setIsOpen(false);
                  }}
                  disabled={isModeLocked}
                />
              </React.Fragment>
            );
          })}
          <Separator size="4" className={styles.separator} />
          <button
            className={styles.addModeItem}
            onClick={handleCreateNewMode}
            type="button"
          >
            <Text size="1">Create new mode...</Text>
          </button>
        </div>
      </Popover.Content>
    </Popover.Root>
  );
};

type ModeMenuItemProps = {
  mode: ChatModeInfo;
  isSelected: boolean;
  onSelect: () => void;
  disabled?: boolean;
};

const ModeMenuItem = React.forwardRef<HTMLButtonElement, ModeMenuItemProps>(
  ({ mode, isSelected, onSelect, disabled }, ref) => {
    return (
      <button
        ref={ref}
        className={`${styles.item} ${isSelected ? styles.itemSelected : ""} ${disabled ? styles.itemDisabled : ""}`}
        onClick={onSelect}
        type="button"
        disabled={disabled}
      >
        <Flex direction="column" gap="1" style={{ width: "100%" }}>
          <Text size="1" weight="medium">{mode.title}</Text>

          {mode.description && (
            <Text size="1" color="gray" className={styles.description}>
              {mode.description.length > 80
                ? mode.description.slice(0, 80) + "..."
                : mode.description}
            </Text>
          )}

          <Flex align="center" gap="1" wrap="wrap">
            {mode.ui.tags.slice(0, 2).map((tag) => (
              <Badge key={tag} size="1" color="gray" variant="soft" className={styles.badge}>
                {tag}
              </Badge>
            ))}
            {mode.tools_count > 0 && (
              <Badge size="1" color="blue" variant="soft" className={styles.badge}>
                {mode.tools_count} tools
              </Badge>
            )}
          </Flex>
        </Flex>
      </button>
    );
  }
);

ModeMenuItem.displayName = "ModeMenuItem";
ModeSelect.displayName = "ModeSelect";
