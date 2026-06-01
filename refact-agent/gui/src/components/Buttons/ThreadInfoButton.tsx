import React, { useCallback, useEffect, useRef, useState } from "react";
import { Box, Flex, Popover, Text } from "@radix-ui/themes";
import { CheckIcon, CopyIcon, InfoCircledIcon } from "@radix-ui/react-icons";
import iconStyles from "./iconButton.module.css";
import { useCopyToClipboard } from "../../hooks";
import {
  useGetTrajectoryPathQuery,
  useLazyGetTrajectoryQuery,
  type TrajectoryData,
} from "../../services/refact";
import { trajectoryDataToChatThread } from "../../services/refact/trajectories";
import { copyChatHistoryToClipboard } from "../../utils/copyChatHistoryToClipboard";
import { useAppSelector } from "../../hooks";
import { selectThreadById } from "../../features/Chat/Thread/selectors";
import styles from "./ThreadInfoButton.module.css";

type ThreadInfoButtonProps = {
  chatId: string | null;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
};

const useCopyStatus = (): {
  copied: boolean;
  trigger: (value: string) => void;
} => {
  const copyToClipboard = useCopyToClipboard();
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    },
    [],
  );

  const trigger = useCallback(
    (value: string) => {
      copyToClipboard(value);
      setCopied(true);
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
      timerRef.current = window.setTimeout(() => {
        setCopied(false);
        timerRef.current = null;
      }, 1500);
    },
    [copyToClipboard],
  );

  return { copied, trigger };
};

const CopyInlineButton: React.FC<{
  value: string | null;
  ariaLabel: string;
  testId: string;
}> = ({ value, ariaLabel, testId }) => {
  const { copied, trigger } = useCopyStatus();
  const handleClick = useCallback(
    (event: React.MouseEvent) => {
      event.stopPropagation();
      if (value) trigger(value);
    },
    [value, trigger],
  );
  return (
    <button
      type="button"
      className={iconStyles.iconButton}
      onClick={handleClick}
      disabled={!value}
      aria-label={ariaLabel}
      data-testid={testId}
    >
      {copied ? (
        <CheckIcon style={{ color: "var(--accent-11)" }} />
      ) : (
        <CopyIcon />
      )}
    </button>
  );
};

export const ThreadInfoButton: React.FC<ThreadInfoButtonProps> = ({
  chatId,
  disabled,
  onOpenChange,
}) => {
  const [open, setOpen] = useState(false);
  const activeThread = useAppSelector((state) =>
    chatId ? selectThreadById(state, chatId) : undefined,
  );

  const isDisabled = disabled === true || !chatId;

  const handleOpenChange = useCallback(
    (next: boolean) => {
      if (isDisabled && next) return;
      setOpen(next);
      onOpenChange?.(next);
    },
    [isDisabled, onOpenChange],
  );

  const pathQuery = useGetTrajectoryPathQuery(chatId ?? "", {
    skip: !chatId || !open,
  });

  const [fetchTrajectory, trajectoryQuery] = useLazyGetTrajectoryQuery();
  const [trajectoryRequested, setTrajectoryRequested] = useState(false);
  const [isCopyingAll, setIsCopyingAll] = useState(false);
  const [copyAllError, setCopyAllError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) {
      setTrajectoryRequested(false);
      setIsCopyingAll(false);
      setCopyAllError(null);
    }
  }, [open]);

  const handleCopyWholeThread = useCallback(
    (event: React.MouseEvent) => {
      event.stopPropagation();
      if (!chatId || isCopyingAll) return;
      setCopyAllError(null);
      if (activeThread && activeThread.messages.length > 0) {
        setIsCopyingAll(true);
        void copyChatHistoryToClipboard({
          ...activeThread,
          model: activeThread.model || "gpt-4o-mini",
        })
          .catch(() => setCopyAllError("Copy failed"))
          .finally(() => setIsCopyingAll(false));
        return;
      }
      setIsCopyingAll(true);
      setTrajectoryRequested(true);
      void (async () => {
        try {
          const data: TrajectoryData = await fetchTrajectory(chatId).unwrap();
          const thread = trajectoryDataToChatThread(data);
          await copyChatHistoryToClipboard({
            ...thread,
            model: thread.model || "gpt-4o-mini",
          });
        } catch {
          setCopyAllError("Copy failed");
        } finally {
          setIsCopyingAll(false);
        }
      })();
    },
    [chatId, activeThread, fetchTrajectory, isCopyingAll],
  );

  const isFetchingTrajectory =
    trajectoryQuery.isLoading || trajectoryQuery.isFetching;
  const isLoadingPath = pathQuery.isLoading || pathQuery.isFetching;
  const pathError = (() => {
    if (!pathQuery.error) return null;
    const status = (pathQuery.error as { status?: number }).status;
    if (status === 404) return "Not saved yet";
    return "Path unavailable";
  })();
  const threadPath = pathQuery.data?.path ?? null;
  const copyAllDisabled =
    !chatId ||
    isLoadingPath ||
    isCopyingAll ||
    (trajectoryRequested && isFetchingTrajectory);

  return (
    <Popover.Root open={open && !isDisabled} onOpenChange={handleOpenChange}>
      <Popover.Trigger>
        <button
          type="button"
          className={iconStyles.iconButton}
          disabled={isDisabled}
          aria-label="Thread info"
          data-testid="thread-info-button"
        >
          <InfoCircledIcon style={{ opacity: 0.75 }} />
        </button>
      </Popover.Trigger>
      <Popover.Content
        side="top"
        align="end"
        sideOffset={8}
        className={styles.popoverContent}
        data-testid="thread-info-popover"
      >
        <Flex direction="column" gap="3" minWidth="280px" maxWidth="420px">
          <Flex direction="column" gap="1">
            <Text size="1" color="gray">
              Thread id
            </Text>
            <Flex align="center" gap="2">
              <Box className={styles.valueText}>
                <Text size="1" className={styles.monoText}>
                  {chatId ?? "—"}
                </Text>
              </Box>
              <CopyInlineButton
                value={chatId}
                ariaLabel="Copy thread id"
                testId="copy-thread-id"
              />
            </Flex>
          </Flex>

          <Flex direction="column" gap="1">
            <Text size="1" color="gray">
              Thread JSON path
            </Text>
            <Flex align="center" gap="2">
              <Box className={styles.valueText}>
                <Text
                  size="1"
                  className={styles.monoText}
                  color={pathError ? "red" : undefined}
                >
                  {pathError ??
                    (isLoadingPath ? "Loading…" : threadPath ?? "—")}
                </Text>
              </Box>
              <CopyInlineButton
                value={threadPath}
                ariaLabel="Copy thread JSON path"
                testId="copy-thread-path"
              />
            </Flex>
          </Flex>

          <Flex direction="column" gap="1">
            <button
              type="button"
              className={styles.copyAllButton}
              onClick={handleCopyWholeThread}
              disabled={copyAllDisabled}
              data-testid="copy-thread-json"
            >
              <CopyIcon />
              <Text size="2">Copy whole thread as JSON</Text>
            </button>
            {copyAllError && (
              <Text size="1" color="red" data-testid="copy-thread-json-error">
                {copyAllError}
              </Text>
            )}
          </Flex>
        </Flex>
      </Popover.Content>
    </Popover.Root>
  );
};

ThreadInfoButton.displayName = "ThreadInfoButton";
