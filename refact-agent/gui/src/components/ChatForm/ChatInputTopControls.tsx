import React, { useCallback, useState } from "react";
import {
  Flex,
  Switch,
  Text,
  Button,
  Tooltip,
  HoverCard,
} from "@radix-ui/themes";
import {
  InfoCircledIcon,
  LockClosedIcon,
  LockOpen1Icon,
  QuestionMarkCircledIcon,
} from "@radix-ui/react-icons";
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
import { TruncateLeft } from "../Text";
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
      <Flex gap="3" align="center" wrap="wrap">
        <Tooltip content="Configure what project information is included in chat context">
          <Button
            variant="ghost"
            size="1"
            onClick={() => setDialogOpen(true)}
            color={includeProjectInfo ? undefined : "gray"}
          >
            <InfoCircledIcon />
            <Text size="1">Project Info</Text>
          </Button>
        </Tooltip>

        <Flex align="center" gap="1">
          <Switch
            size="1"
            checked={autoApproveEditing}
            onCheckedChange={handleEditingChange}
          />
          <Tooltip content="Automatically approve file editing tools (patch, create, update, mv)">
            <Text size="1">Auto-approve edits</Text>
          </Tooltip>
        </Flex>

        <Flex align="center" gap="1">
          <Switch
            size="1"
            checked={autoApproveDangerous}
            onCheckedChange={handleDangerousChange}
          />
          <Tooltip content="Automatically approve dangerous commands (shell, rm). Use with caution!">
            <Text size="1" color={autoApproveDangerous ? "red" : undefined}>
              Auto-approve dangerous
            </Text>
          </Tooltip>
        </Flex>

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
          <Button
            variant="ghost"
            size="1"
            onClick={attachedFiles.addFile}
            disabled={attachedFiles.attached}
          >
            <Text size="1">
              Attach:{" "}
              <TruncateLeft>{attachedFiles.activeFile.name}</TruncateLeft>
            </Text>
          </Button>
        )}
      </Flex>

      <ProjectInformationDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
    </>
  );
};
