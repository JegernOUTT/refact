import React, { useCallback, useState } from "react";
import {
  CircleHelp,
  Info,
  Lock,
  Pencil,
  Plus,
  TriangleAlert,
  Unlock,
} from "lucide-react";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectAutoApproveEditingToolsById,
  selectAutoApproveDangerousCommandsById,
  selectIncludeProjectInfoById,
  useThreadId,
} from "../../features/Chat/Thread";
import {
  setAutoApproveEditingTools,
  setAutoApproveDangerousCommands,
} from "../../features/Chat/Thread/actions";
import { ProjectInformationDialog } from "./ProjectInformationDialog";
import { selectHost } from "../../features/Config/configSlice";
import { Checkbox } from "../Checkbox";
import { IconButton, Tooltip } from "../ui";
import type { Checkbox as CheckboxType } from "./useCheckBoxes";
import type { useAttachedFiles } from "./useCheckBoxes";
import styles from "./ChatInputTopControls.module.css";

export type ChatInputTopControlsProps = {
  checkboxes: Record<string, CheckboxType>;
  onCheckedChange: (name: string, checked: boolean | string) => void;
  attachedFiles: ReturnType<typeof useAttachedFiles>;
  disabled?: boolean;
};

export const ChatInputTopControls: React.FC<ChatInputTopControlsProps> = ({
  checkboxes,
  onCheckedChange,
  attachedFiles,
  disabled,
}) => {
  const isDisabled = disabled ?? false;
  const dispatch = useAppDispatch();
  const host = useAppSelector(selectHost);
  const chatId = useThreadId();
  const autoApproveEditing = useAppSelector((state) =>
    selectAutoApproveEditingToolsById(state, chatId),
  );
  const autoApproveDangerous = useAppSelector((state) =>
    selectAutoApproveDangerousCommandsById(state, chatId),
  );
  const includeProjectInfo = useAppSelector((state) =>
    selectIncludeProjectInfoById(state, chatId),
  );
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
      <div className={styles.controlsGroup}>
        <span className={styles.projectInfoControl}>
          <Tooltip>
            <Tooltip.Trigger asChild>
              <IconButton
                icon={Info}
                variant="plain"
                size="sm"
                type="button"
                aria-label="Configure project information"
                disabled={isDisabled}
                className={includeProjectInfo ? styles.accent : undefined}
                onClick={() => setDialogOpen(true)}
              />
            </Tooltip.Trigger>
            <Tooltip.Content>
              Project info: {includeProjectInfo ? "ON" : "OFF"}
            </Tooltip.Content>
          </Tooltip>
        </span>

        <Tooltip>
          <Tooltip.Trigger asChild>
            <IconButton
              icon={Pencil}
              variant="plain"
              size="sm"
              type="button"
              aria-label="Auto-approve file editing tools"
              aria-pressed={autoApproveEditing}
              disabled={isDisabled || !chatId}
              className={autoApproveEditing ? styles.accent : undefined}
              onClick={() => handleEditingChange(!autoApproveEditing)}
            />
          </Tooltip.Trigger>
          <Tooltip.Content>
            Auto-approve edits: {autoApproveEditing ? "ON" : "OFF"}
          </Tooltip.Content>
        </Tooltip>

        <Tooltip>
          <Tooltip.Trigger asChild>
            <IconButton
              icon={TriangleAlert}
              variant="plain"
              size="sm"
              type="button"
              aria-label="Auto-approve dangerous commands"
              aria-pressed={autoApproveDangerous}
              disabled={isDisabled || !chatId}
              className={autoApproveDangerous ? styles.dangerIcon : undefined}
              onClick={() => handleDangerousChange(!autoApproveDangerous)}
            />
          </Tooltip.Trigger>
          <Tooltip.Content>
            Auto-approve dangerous: {autoApproveDangerous ? "ON" : "OFF"}
          </Tooltip.Content>
        </Tooltip>

        {showSelectedLines && (
          <>
            <span className={styles.divider}>|</span>
            <div className={styles.selectedLinesGroup}>
              <Checkbox
                size="1"
                name={selectedLinesCheckbox.name}
                checked={selectedLinesCheckbox.checked}
                disabled={isDisabled || selectedLinesCheckbox.disabled}
                onCheckedChange={(value) =>
                  onCheckedChange(selectedLinesCheckbox.name, value)
                }
              >
                <span>{selectedLinesCheckbox.label}</span>
              </Checkbox>
              <IconButton
                icon={selectedLinesCheckbox.locked ? Lock : Unlock}
                variant="plain"
                size="sm"
                type="button"
                aria-label={
                  selectedLinesCheckbox.locked ? "Locked" : "Unlocked"
                }
                disabled={isDisabled || selectedLinesCheckbox.disabled}
                onClick={() =>
                  onCheckedChange(
                    selectedLinesCheckbox.name,
                    !selectedLinesCheckbox.checked,
                  )
                }
              />
              {selectedLinesCheckbox.info && (
                <Tooltip>
                  <Tooltip.Trigger asChild>
                    <IconButton
                      icon={CircleHelp}
                      variant="plain"
                      size="sm"
                      type="button"
                      aria-label="Selected lines information"
                      disabled={isDisabled}
                    />
                  </Tooltip.Trigger>
                  <Tooltip.Content maxWidth="240px">
                    {selectedLinesCheckbox.info.text}
                  </Tooltip.Content>
                </Tooltip>
              )}
            </div>
          </>
        )}

        {showAttachButton && (
          <>
            <span className={styles.divider}>|</span>
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  icon={Plus}
                  variant="plain"
                  size="sm"
                  type="button"
                  aria-label={`Attach ${attachedFiles.activeFile.name}`}
                  disabled={isDisabled || attachedFiles.attached}
                  className={attachedFiles.attached ? styles.accent : undefined}
                  onClick={attachedFiles.addFile}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>
                Attach: {attachedFiles.activeFile.name}
              </Tooltip.Content>
            </Tooltip>
          </>
        )}
      </div>

      <ProjectInformationDialog
        chatId={chatId}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
    </>
  );
};
