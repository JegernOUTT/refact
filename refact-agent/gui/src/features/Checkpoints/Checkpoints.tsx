import { useState } from "react";
import classNames from "classnames";
import { Dialog, Button, SegmentedControl } from "../../components/ui";
import { useCheckpoints, useOpenFileInApp } from "../../hooks";
import { TruncateLeft } from "../../components/Text";
import { Link } from "../../components/Link";
import { ScrollArea } from "../../components/ScrollArea";

import styles from "./Checkpoints.module.css";
import { formatDateOrTimeBasedOnToday } from "../../utils/formatDateToHumanReadable";
import { formatPathName } from "../../utils/formatPathName";
import { CheckpointsStatusIndicator } from "./CheckpointsStatusIndicator";
import { ErrorCallout } from "../../components/Callout";

export type RestoreMode = "files_only" | "files_and_messages";

export const Checkpoints = () => {
  const { canOpen, openFile } = useOpenFileInApp();
  const {
    shouldCheckpointsPopupBeShown,
    handleFix,
    handleUndo,
    reverted_to,
    isRestoring,
    allChangedFiles,
    wereFilesChanged,
    errorLog,
  } = useCheckpoints();

  const [restoreMode, setRestoreMode] =
    useState<RestoreMode>("files_and_messages");

  const clientTimezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const formattedDate = formatDateOrTimeBasedOnToday(
    reverted_to,
    clientTimezone,
  );

  const checkpointsTitle = `${
    wereFilesChanged ? "Files changed" : "No files were changed"
  } from checkpoint at ${formattedDate}`;

  return (
    <Dialog
      open={shouldCheckpointsPopupBeShown}
      onOpenChange={(state) => {
        if (!state) {
          handleUndo();
        }
      }}
    >
      <Dialog.Content className={styles.CheckpointsDialog}>
        <Dialog.Description>
          Restores chat and your project&apos;s files back to a snapshot taken
          at this point
        </Dialog.Description>
        <Dialog.Title>
          {errorLog.length >= 1
            ? "Oops... Something went wrong"
            : checkpointsTitle}
        </Dialog.Title>
        <ScrollArea scrollbars="vertical" className={styles.fileScroll}>
          <div className={classNames(styles.fileList, "rf-stagger")}>
            {wereFilesChanged &&
              allChangedFiles.map((file, index) => {
                const formattedWorkspaceFolder = formatPathName(
                  file.workspace_folder,
                );
                return (
                  <div
                    key={`${file.absolute_path}-${index}`}
                    className={classNames(styles.fileRow, "rf-enter-rise")}
                  >
                    <div className={styles.fileInfo}>
                      <TruncateLeft size="2" className={styles.filePathWrap}>
                        {canOpen ? (
                          <Link
                            title="Open file"
                            onClick={(event) => {
                              event.preventDefault();
                              openFile({
                                path: file.absolute_path,
                                resolved: true,
                              });
                            }}
                            className={
                              file.status === "DELETED"
                                ? styles.deletedLink
                                : undefined
                            }
                          >
                            {formatPathName(file.absolute_path)}
                          </Link>
                        ) : (
                          <span
                            className={
                              file.status === "DELETED"
                                ? styles.deletedLink
                                : undefined
                            }
                          >
                            {formatPathName(file.absolute_path)}
                          </span>
                        )}
                      </TruncateLeft>
                      <span className={styles.workspaceFolder}>
                        {formattedWorkspaceFolder}
                      </span>

                      <CheckpointsStatusIndicator status={file.status} />
                    </div>
                  </div>
                );
              })}
          </div>
        </ScrollArea>
        {errorLog.length > 0 && (
          <ErrorCallout mx="0" preventRetry>
            {errorLog.join("\n")}
          </ErrorCallout>
        )}

        <div className={styles.restoreOptions}>
          <span className={styles.optionTitle}>Restore options:</span>
          <SegmentedControl
            size="sm"
            value={restoreMode}
            onValueChange={(value) => setRestoreMode(value as RestoreMode)}
            options={[
              {
                value: "files_and_messages",
                label: "Files + messages",
              },
              {
                value: "files_only",
                label: "Files only",
              },
            ]}
          />
        </div>

        <div className={styles.actions}>
          <Button type="button" variant="soft" onClick={handleUndo}>
            Cancel
          </Button>
          <Button
            variant="primary"
            loading={isRestoring}
            disabled={errorLog.length > 0}
            onClick={() => void handleFix(restoreMode)}
            title={
              isRestoring
                ? "Rolling back..."
                : errorLog.length > 0
                  ? "There are some errors, you cannot roll back to this checkpoint"
                  : "Roll back to checkpoint"
            }
          >
            Roll back to checkpoint
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
