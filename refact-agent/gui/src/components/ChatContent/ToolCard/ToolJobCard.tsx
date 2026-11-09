import { useState } from "react";
import { Clock3 } from "lucide-react";
import { useAppSelector } from "../../../hooks/useAppSelector";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import {
  isTerminalToolJob,
  useCancelToolJobMutation,
  useGetToolJobQuery,
  type ToolJobReceipt,
  type ToolJobStatus,
} from "../../../services/refact/toolJobs";
import type { ToolCall } from "../../../services/refact/types";
import { ToolCard } from "./ToolCard";
import styles from "./ToolJobCard.module.css";

const labels: Record<ToolJobStatus, string> = {
  accepted: "Accepted",
  running: "Running in background",
  cancel_requested: "Cancellation requested",
  succeeded: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
  unknown: "Outcome unknown",
};

export function ToolJobCompletionCard({
  receipt,
}: {
  receipt: ToolJobReceipt;
}) {
  if (!isTerminalToolJob(receipt.status)) return null;
  return (
    <ToolCard
      icon={<Clock3 size={14} />}
      summary={receipt.tool_name}
      verb="Background job"
      density="compact"
      status={
        receipt.status === "failed" || receipt.status === "unknown"
          ? "error"
          : "idle"
      }
      statusLabel={labels[receipt.status]}
      isOpen={false}
      onToggle={() => undefined}
      collapsible={false}
    />
  );
}

export function ToolJobCard({
  receipt,
  toolCall,
}: {
  receipt: ToolJobReceipt;
  toolCall: ToolCall;
}) {
  const endpoint = useAppSelector((state) =>
    getEngineEndpointIdentity(state.config),
  );
  const apiKey = useAppSelector((state) => state.config.apiKey);
  const [source] = useState({ endpoint, apiKey });
  const sourceChanged =
    source.endpoint !== endpoint || source.apiKey !== apiKey;
  const [open, setOpen] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [cancelError, setCancelError] = useState(false);
  const args = { chatId: receipt.chat_id, jobId: receipt.job_id, endpoint };
  const [cancel, cancelState] = useCancelToolJobMutation();
  const [terminal, setTerminal] = useState(false);
  const { currentData, isError, refetch } = useGetToolJobQuery(args, {
    pollingInterval: terminal ? 0 : 3000,
    skipPollingIfUnfocused: true,
    skip: sourceChanged,
  });
  const matching =
    currentData &&
    currentData.receipt.tool_call_id === receipt.tool_call_id &&
    currentData.receipt.tool_name === receipt.tool_name &&
    currentData.receipt.turn_id === receipt.turn_id &&
    currentData.receipt.model === receipt.model;
  const status = matching ? currentData.status : receipt.status;
  if (!terminal && matching && isTerminalToolJob(status)) setTerminal(true);
  const visibleStatus =
    cancelRequested && !isTerminalToolJob(status) ? "cancel_requested" : status;
  const onCancel = async () => {
    setCancelError(false);
    try {
      const response = await cancel(args).unwrap();
      setCancelRequested(response.status === "cancel_requested");
      void refetch();
    } catch {
      setCancelError(true);
    }
  };
  return (
    <ToolCard
      icon={<Clock3 size={14} />}
      summary={receipt.tool_name}
      verb="Background job"
      density="compact"
      status={
        visibleStatus === "failed" || visibleStatus === "unknown"
          ? "error"
          : "idle"
      }
      statusLabel={
        sourceChanged || isError ? "Status unavailable" : labels[visibleStatus]
      }
      isOpen={open}
      onToggle={() => setOpen((value) => !value)}
      toolCall={toolCall}
      actions={
        !isTerminalToolJob(status) ? (
          <button
            type="button"
            className={styles.action}
            disabled={
              sourceChanged ||
              isError ||
              !matching ||
              cancelRequested ||
              cancelState.isLoading ||
              status === "cancel_requested"
            }
            onClick={() => void onCancel()}
            aria-label={`Cancel ${receipt.tool_name} background job`}
          >
            Cancel
          </button>
        ) : undefined
      }
    >
      <div className={styles.detail}>
        {cancelError
          ? "Cancellation could not be confirmed."
          : visibleStatus === "unknown"
            ? "The outcome is unknown; do not automatically retry."
            : isError
              ? "Unable to check job status."
              : "Results appear in the conversation when available."}
      </div>
    </ToolCard>
  );
}
