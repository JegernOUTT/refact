import { useState } from "react";
import { Text } from "@radix-ui/themes";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectReviewReadinessById } from "../../features/Chat/Thread/selectors";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import {
  isTerminalToolJob,
  useCancelToolJobMutation,
  useGetToolJobQuery,
  type ToolJobReceipt,
} from "../../services/refact/toolJobs";
import { Button, StatusDot } from "../ui";
import styles from "./ReviewReadinessBanner.module.css";

function PendingReview({ receipt }: { receipt: ToolJobReceipt }) {
  const endpoint = useAppSelector((state) =>
    getEngineEndpointIdentity(state.config),
  );
  const apiKey = useAppSelector((state) => state.config.apiKey);
  const [source] = useState({ endpoint, apiKey });
  const changed = source.endpoint !== endpoint || source.apiKey !== apiKey;
  const [cancel, cancellation] = useCancelToolJobMutation();
  const [requested, setRequested] = useState(false);
  const [cancelError, setCancelError] = useState(false);
  const [finished, setFinished] = useState(false);
  const args = { chatId: receipt.chat_id, jobId: receipt.job_id, endpoint };
  const { currentData, isError } = useGetToolJobQuery(args, {
    skip: changed,
    pollingInterval: finished ? 0 : 3000,
    skipPollingIfUnfocused: true,
  });
  const matching =
    !changed &&
    currentData?.receipt.tool_call_id === receipt.tool_call_id &&
    currentData.receipt.turn_id === receipt.turn_id &&
    currentData.receipt.tool_name === receipt.tool_name &&
    currentData.receipt.model === receipt.model;
  const status = matching ? currentData.status : receipt.status;
  const unavailable = changed || isError;
  const terminal = isTerminalToolJob(status);
  if (!finished && matching && terminal) setFinished(true);
  const label = unavailable
    ? "Review preparation status unavailable"
    : status === "cancelled"
      ? "Review preparation cancelled — you can send a new message"
      : status === "failed" || status === "unknown"
        ? "Review preparation unavailable — you can send a new message"
        : requested || status === "cancel_requested"
          ? "Review preparation cancellation requested"
          : status === "succeeded"
            ? "Review preparation finished — waiting for context attachment"
            : "Preparing review… You can keep writing";
  const onCancel = async () => {
    setCancelError(false);
    try {
      const response = await cancel(args).unwrap();
      setRequested(response.status === "cancel_requested");
    } catch {
      setCancelError(true);
    }
  };
  return (
    <div className={styles.banner} role="status" aria-live="polite">
      <StatusDot status="idle" pulse={!unavailable && !terminal} aria-hidden />
      <Text size="1">{label}</Text>
      {!terminal && (
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={
            unavailable ||
            !matching ||
            requested ||
            cancellation.isLoading ||
            status === "cancel_requested"
          }
          onClick={() => void onCancel()}
        >
          Cancel review preparation
        </Button>
      )}
      {cancelError && (
        <Text size="1">Cancellation could not be confirmed.</Text>
      )}
    </div>
  );
}

export function ReviewReadinessBanner({ chatId }: { chatId: string }) {
  const readiness = useAppSelector((state) =>
    selectReviewReadinessById(state, chatId),
  );
  if (!readiness || readiness.status === "ready") return null;
  return (
    <div data-composer-no-expand="true" data-testid="review-readiness-banner">
      {readiness.status === "pending" ? (
        <PendingReview
          key={`${chatId}:${readiness.turn_nonce}:${readiness.receipt.job_id}`}
          receipt={readiness.receipt}
        />
      ) : (
        <div className={styles.banner} role="status" aria-live="polite">
          <StatusDot status="idle" aria-hidden />
          <Text size="1">
            Review preparation unavailable — you can send a new message
          </Text>
        </div>
      )}
    </div>
  );
}
