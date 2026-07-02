import React, { useCallback, useState } from "react";
import { Check, Copy, ExternalLink } from "lucide-react";
import { Flex, Button, Icon, Text } from "../../components/ui";
import { useAppSelector, useCopyToClipboard } from "../../hooks";
import { selectCurrentThreadId, selectThread } from "../Chat/Thread/selectors";
import {
  useGetTrajectoryPathQuery,
  useLazyGetTrajectoryQuery,
  type TrajectoryData,
} from "../../services/refact";
import { trajectoryDataToChatThread } from "../../services/refact/trajectories";
import { copyChatHistoryToClipboard } from "../../utils/copyChatHistoryToClipboard";
import { reportBuddyFrontendError } from "./reportBuddyFrontendError";
import styles from "./BuddyErrorBoundary.module.css";

type Props = {
  children?: React.ReactNode;
  showThreadReportPanel?: boolean;
};

type State = {
  failed: boolean;
  errorDetails: string | null;
};

function issueUrl(threadId: string | null): string {
  const title = threadId
    ? `Frontend crash in thread ${threadId}`
    : "Frontend crash in Refact chat";
  const body = threadId
    ? `The chat frontend crashed while rendering thread \`${threadId}\`.\n\nPlease paste the copied thread JSON here if it is safe to share.`
    : "The chat frontend crashed. Please paste the copied thread JSON here if it is safe to share.";

  const params = new URLSearchParams({ title, body });
  return `https://github.com/JegernOUTT/refact/issues/new?${params.toString()}`;
}

const CopyInlineButton: React.FC<{
  value: string | null;
  ariaLabel: string;
  testId: string;
}> = ({ value, ariaLabel, testId }) => {
  const copyToClipboard = useCopyToClipboard();
  const [copied, setCopied] = useState(false);

  const handleClick = useCallback(() => {
    if (!value) return;
    copyToClipboard(value);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }, [copyToClipboard, value]);

  return (
    <button
      type="button"
      className={styles.copyButton}
      onClick={handleClick}
      disabled={!value}
      aria-label={ariaLabel}
      data-testid={testId}
    >
      <Icon icon={copied ? Check : Copy} size="sm" />
    </button>
  );
};

const CrashField: React.FC<{
  label: string;
  value: string | null;
  copyValue?: string | null;
  placeholder?: string;
  error?: boolean;
  copyLabel: string;
  testId: string;
}> = ({
  label,
  value,
  copyValue,
  placeholder = "—",
  error = false,
  copyLabel,
  testId,
}) => (
  <Flex direction="column" gap="1">
    <Text size="2" color="gray" weight="medium">
      {label}
    </Text>
    <Flex align="center" gap="2">
      <div className={styles.valueText}>
        <Text
          size="2"
          className={styles.monoText}
          color={error ? "red" : undefined}
        >
          {value ?? placeholder}
        </Text>
      </div>
      <CopyInlineButton
        value={copyValue === undefined ? value : copyValue}
        ariaLabel={copyLabel}
        testId={testId}
      />
    </Flex>
  </Flex>
);

const ErrorDetailsField: React.FC<{ details: string | null }> = ({
  details,
}) => (
  <Flex direction="column" gap="1">
    <Text size="2" color="gray" weight="medium">
      Error details
    </Text>
    <Flex align="start" gap="2">
      <textarea
        readOnly
        className={styles.errorDetailsField}
        value={details ?? "No frontend error details were captured."}
        aria-label="Frontend error details"
        data-testid="crash-error-details"
      />
      <CopyInlineButton
        value={details}
        ariaLabel="Copy frontend error details"
        testId="copy-crash-error-details"
      />
    </Flex>
  </Flex>
);

const ChatCrashReportPanel: React.FC<{ errorDetails: string | null }> = ({
  errorDetails,
}) => {
  const chatId = useAppSelector(selectCurrentThreadId) || null;
  const activeThread = useAppSelector(selectThread);
  const [fetchTrajectory, trajectoryQuery] = useLazyGetTrajectoryQuery();
  const [isCopyingAll, setIsCopyingAll] = useState(false);
  const [copyAllError, setCopyAllError] = useState<string | null>(null);

  const pathQuery = useGetTrajectoryPathQuery(chatId ?? "", {
    skip: !chatId,
  });

  const isLoadingPath = pathQuery.isLoading || pathQuery.isFetching;
  const pathStatus = (() => {
    if (!pathQuery.error) return null;
    const status = (pathQuery.error as { status?: number }).status;
    if (status === 404) return "Not saved yet";
    return "Path unavailable";
  })();
  const threadPath = pathQuery.data?.path ?? null;
  const pathValue = pathStatus ?? (isLoadingPath ? "Loading…" : threadPath);
  const copyablePathValue = pathStatus || isLoadingPath ? null : threadPath;
  const canCopyWholeThread = Boolean(chatId) && !isCopyingAll;
  const isFetchingTrajectory =
    trajectoryQuery.isLoading || trajectoryQuery.isFetching;

  const handleRefresh = useCallback(() => {
    window.location.reload();
  }, []);

  const handleCopyWholeThread = useCallback(() => {
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
  }, [activeThread, chatId, fetchTrajectory, isCopyingAll]);

  return (
    <Flex align="center" justify="center" className={styles.root}>
      <div className={styles.card} role="alert">
        <Flex direction="column" gap="2">
          <Text size="4" weight="bold">
            The chat frontend crashed.
          </Text>
          <Text size="2" color="gray">
            Refresh the view to recover. If it happens again, please open an
            issue and include this thread context so we can reproduce it.
          </Text>
        </Flex>

        <Flex direction="column" gap="3" className={styles.reportPanel}>
          <CrashField
            label="Thread id"
            value={chatId}
            copyLabel="Copy thread id"
            testId="copy-crash-thread-id"
          />
          <CrashField
            label="Thread JSON path"
            value={pathValue}
            copyValue={copyablePathValue}
            error={Boolean(pathStatus)}
            copyLabel="Copy thread JSON path"
            testId="copy-crash-thread-path"
          />
          <ErrorDetailsField details={errorDetails} />

          <button
            type="button"
            className={styles.copyAllButton}
            onClick={handleCopyWholeThread}
            disabled={!canCopyWholeThread || isFetchingTrajectory}
            data-testid="copy-crash-thread-json"
          >
            <Icon icon={Copy} size="sm" />
            <Text size="2">
              {isCopyingAll || isFetchingTrajectory
                ? "Copying thread JSON…"
                : "Copy whole thread as JSON"}
            </Text>
          </button>
          {copyAllError && (
            <Text size="1" data-testid="copy-crash-thread-json-error">
              {copyAllError}
            </Text>
          )}
        </Flex>

        <Flex gap="2" wrap="wrap" justify="center">
          <Button type="button" variant="danger" onClick={handleRefresh}>
            Refresh
          </Button>
          <Button asChild variant="outline">
            <a href={issueUrl(chatId)} target="_blank" rel="noreferrer">
              <Icon icon={ExternalLink} size="sm" />
              Open GitHub issue
            </a>
          </Button>
        </Flex>
      </div>
    </Flex>
  );
};

export class BuddyErrorBoundary extends React.Component<Props, State> {
  override state: State = {
    failed: false,
    errorDetails: null,
  };

  static getDerivedStateFromError(error: Error): State {
    return {
      failed: true,
      errorDetails: error.stack ?? error.message,
    };
  }

  override componentDidCatch(error: Error, errorInfo: React.ErrorInfo): void {
    const details = errorInfo.componentStack
      ? `${error.stack ?? error.message}\n\nComponent stack:\n${
          errorInfo.componentStack
        }`
      : error.stack ?? error.message;

    this.setState({ errorDetails: details });

    void reportBuddyFrontendError({
      source: "react_error_boundary",
      error: details,
      sourceFile: "frontend/react_error_boundary",
      toolName: "react_error_boundary",
    });
  }

  override render(): React.ReactNode {
    if (this.state.failed) {
      if (this.props.showThreadReportPanel === true) {
        return <ChatCrashReportPanel errorDetails={this.state.errorDetails} />;
      }

      return (
        <Flex align="center" justify="center" className={styles.root}>
          <div className={styles.card} role="alert">
            <Text size="3" weight="bold">
              The app hit a frontend error.
            </Text>
            <Text size="2" color="gray">
              Your companion recorded it for investigation. Reload the view if
              it stays blank.
            </Text>
          </div>
        </Flex>
      );
    }

    return this.props.children;
  }
}

export function withBuddyErrorReport<T>(
  fn: () => T,
  args: {
    source: "react_root_render" | "react_recoverable";
    sourceFile: string;
    toolName: string;
  },
): T {
  try {
    return fn();
  } catch (error) {
    void reportBuddyFrontendError({
      source: args.source,
      error,
      sourceFile: args.sourceFile,
      toolName: args.toolName,
    });
    throw error;
  }
}
