import React, { useCallback, useState } from "react";
import { Flex, Switch, Text, Button, Tooltip } from "@radix-ui/themes";
import { InfoCircledIcon } from "@radix-ui/react-icons";
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

export const ChatInputTopControls: React.FC = () => {
  const dispatch = useAppDispatch();
  const chatId = useAppSelector(selectCurrentThreadId);
  const autoApproveEditing = useAppSelector(selectAutoApproveEditingTools);
  const autoApproveDangerous = useAppSelector(selectAutoApproveDangerousCommands);
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

  return (
    <>
      <Flex gap="4" align="center" wrap="wrap" px="2" py="1">
        <Tooltip content="Configure what project information is included in chat context">
          <Button
            variant="ghost"
            size="1"
            onClick={() => setDialogOpen(true)}
            color={includeProjectInfo ? undefined : "gray"}
          >
            <InfoCircledIcon />
            Project Info
          </Button>
        </Tooltip>

        <Flex align="center" gap="2">
          <Switch
            size="1"
            checked={autoApproveEditing}
            onCheckedChange={handleEditingChange}
          />
          <Tooltip content="Automatically approve file editing tools (patch, create, update, mv)">
            <Text size="1">Auto-approve edits</Text>
          </Tooltip>
        </Flex>

        <Flex align="center" gap="2">
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
      </Flex>

      <ProjectInformationDialog open={dialogOpen} onOpenChange={setDialogOpen} />
    </>
  );
};
