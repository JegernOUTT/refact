import React, { useCallback, useState } from "react";
import { Flex, Text, HoverCard } from "@radix-ui/themes";
import {
  InfoCircledIcon,
  LockClosedIcon,
  LockOpen1Icon,
  QuestionMarkCircledIcon,
  Pencil2Icon,
  ExclamationTriangleIcon,
  PlusIcon,
} from "@radix-ui/react-icons";
import styles from "./ChatInputTopControls.module.css";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectAutoApproveEditingTools,
  selectAutoApproveDangerousCommands,
  selectCurrentThreadId,
  selectIncludeProjectInfo,
} from "../../features/Chat";
import {
  setAutoApproveEditingTools,
  setAutoApproveDangerousCommands,
} from "../../features/Chat/Thread/actions";
import { ProjectInformationDialog } from "./ProjectInformationDialog";
import { selectHost } from "../../features/Config/configSlice";
import { Checkbox } from "../Checkbox";
import type { Checkbox as CheckboxType } from "./useCheckBoxes";
import type { useAttachedFiles } from "./useCheckBoxes";

export type ChatInputTopControlsProps = {
  checkboxes: Record<string, CheckboxType>;
  onCheckedChange: (name: string, checked: boolean | string) => void;
  attachedFiles: ReturnType<typeof useAttachedFiles>;
};

export const ChatInputTopControls: React.FC<ChatInputTopControlsProps> = ({
  checkboxes,
  onCheckedChange,
  attachedFiles,
}) => {
  const dispatch = useAppDispatch();
  const host = useAppSelector(selectHost);
  const chatId = useAppSelector(selectCurrentThreadId);
  const autoApproveEditing = useAppSelector(selectAutoApproveEditingTools);
  const autoApproveDangerous = useAppSelector(
    selectAutoApproveDangerousCommands,
  );
  const includeProjectInfo = useAppSelector(selectIncludeProjectInfo);
  const [dialogOpen, setDialogOpen] = useState(false);

  const handleEditingChange = useCallback(
    (checked: boolean) => {
      if (chatId) {
        dispatch(setAutoApproveEditingTools({ chatId, value: checked }));
      }
    },
    [dispatch, chatId],
  );

  const handleDangerousChange = useCallback(
    (checked: boolean) => {
      if (chatId) {
        dispatch(setAutoApproveDangerousCommands({ chatId, value: checked }));
      }
    },
    [dispatch, chatId],
  );

  const selectedLinesCheckbox = checkboxes.selected_lines;
  const showSelectedLines = host !== "web" && !selectedLinesCheckbox.hide;
  const showAttachButton = host !== "web" && attachedFiles.activeFile.name;

  return (
    <>
      <Flex gap="1" align="center" wrap="wrap">
        <HoverCard.Root>
          <HoverCard.Trigger>
            <button
              type="button"
              onClick={() => setDialogOpen(true)}
              aria-label="Configure project information"
              className={classNames(
                styles.iconButton,
                includeProjectInfo && styles.active,
              )}
            >
              <InfoCircledIcon />
            </button>
          </HoverCard.Trigger>
          <HoverCard.Content size="1" side="top">
            <Text as="p" size="2">
              Project info: {includeProjectInfo ? "ON" : "OFF"}
            </Text>
          </HoverCard.Content>
        </HoverCard.Root>

        <HoverCard.Root>
          <HoverCard.Trigger>
            <button
              type="button"
              onClick={() => handleEditingChange(!autoApproveEditing)}
              disabled={!chatId}
              aria-label="Auto-approve file editing tools"
              aria-pressed={autoApproveEditing}
              className={classNames(
                styles.iconButton,
                autoApproveEditing && styles.active,
              )}
            >
              <Pencil2Icon />
            </button>
          </HoverCard.Trigger>
          <HoverCard.Content size="1" side="top">
            <Text as="p" size="2">
              Auto-approve edits: {autoApproveEditing ? "ON" : "OFF"}
            </Text>
          </HoverCard.Content>
        </HoverCard.Root>

        <HoverCard.Root>
          <HoverCard.Trigger>
            <button
              type="button"
              onClick={() => handleDangerousChange(!autoApproveDangerous)}
              disabled={!chatId}
              aria-label="Auto-approve dangerous commands"
              aria-pressed={autoApproveDangerous}
              className={classNames(
                styles.iconButton,
                autoApproveDangerous && styles.danger,
              )}
            >
              <ExclamationTriangleIcon />
            </button>
          </HoverCard.Trigger>
          <HoverCard.Content size="1" side="top">
            <Text as="p" size="2">
              Auto-approve dangerous: {autoApproveDangerous ? "ON" : "OFF"}
            </Text>
          </HoverCard.Content>
        </HoverCard.Root>

        {showSelectedLines && (
          <Flex align="center" gap="1">
            <Checkbox
              size="1"
              name={selectedLinesCheckbox.name}
              checked={selectedLinesCheckbox.checked}
              disabled={selectedLinesCheckbox.disabled}
              onCheckedChange={(value) =>
                onCheckedChange(selectedLinesCheckbox.name, value)
              }
            >
              <Text size="1">{selectedLinesCheckbox.label}</Text>
              {selectedLinesCheckbox.locked && <LockClosedIcon opacity="0.6" />}
              {selectedLinesCheckbox.locked === false && (
                <LockOpen1Icon opacity="0.6" />
              )}
            </Checkbox>
            {selectedLinesCheckbox.info && (
              <HoverCard.Root>
                <HoverCard.Trigger>
                  <QuestionMarkCircledIcon
                    style={{ cursor: "help", opacity: 0.6 }}
                  />
                </HoverCard.Trigger>
                <HoverCard.Content maxWidth="240px" size="1">
                  <Text as="div" size="1">
                    {selectedLinesCheckbox.info.text}
                  </Text>
                </HoverCard.Content>
              </HoverCard.Root>
            )}
          </Flex>
        )}

        {showAttachButton && (
          <HoverCard.Root>
            <HoverCard.Trigger>
              <button
                type="button"
                onClick={attachedFiles.addFile}
                disabled={attachedFiles.attached}
                aria-label={`Attach ${attachedFiles.activeFile.name}`}
                className={classNames(
                  styles.iconButton,
                  attachedFiles.attached && styles.active,
                )}
              >
                <PlusIcon />
              </button>
            </HoverCard.Trigger>
            <HoverCard.Content size="1" side="top">
              <Text as="p" size="2">
                Attach: {attachedFiles.activeFile.name}
              </Text>
            </HoverCard.Content>
          </HoverCard.Root>
        )}
      </Flex>

      <ProjectInformationDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
    </>
  );
};
