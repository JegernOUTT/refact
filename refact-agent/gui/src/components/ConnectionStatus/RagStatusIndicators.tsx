import React from "react";

import { useAppSelector } from "../../hooks/useAppSelector";
import { selectConfig } from "../../features/Config/configSlice";
import { hasUsableEngineEndpoint } from "../../services/refact/apiUrl";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import type {
  CodeGraphState,
  CodeGraphStatus,
  RagStatus,
} from "../../services/refact/types";
import { MiniStatusIndicator } from "./ConnectionStatusIndicator";
import styles from "./ConnectionStatus.module.css";

type IndicatorState = React.ComponentProps<
  typeof MiniStatusIndicator
>["status"];

function stateFromAlive(value: string): IndicatorState {
  if (value === "working") return "success";
  if (value === "indexing") return "working";
  if (value === "turned_off") return "idle";
  if (value === "") return "idle";
  return "error";
}

function codegraphIndicatorState(
  alive: string,
  status: CodeGraphStatus | null,
): IndicatorState {
  const state: CodeGraphState | null = status?.state ?? null;
  if (state === "indexing") return "working";
  if (state === "working") return "success";
  if (state === "turned_off") return "idle";
  if (state === "error") return "error";
  return stateFromAlive(alive);
}

function formatCodegraphTooltip(status: RagStatus): string {
  const codegraph = status.codegraph;
  const alive = status.codegraph_alive;
  if (!codegraph) return `CodeGraph: ${alive}`;
  const suffix = codegraph.error ? ` · ${codegraph.error}` : "";
  return `CodeGraph: ${alive} · ${codegraph.counts.files} files · ${codegraph.counts.nodes} nodes · ${codegraph.queued} queued${suffix}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function formatRagStatusError(error: unknown): string {
  if (!isRecord(error)) return "latest status poll failed";
  if (typeof error.error === "string") return error.error;
  if (typeof error.data === "string") return error.data;
  if (typeof error.message === "string") return error.message;
  if ("status" in error) return `status ${String(error.status)}`;
  return "latest status poll failed";
}

function formatCodegraphErrorTooltip(
  status: RagStatus | undefined,
  error: unknown,
): string {
  const staleStatus = status
    ? `${formatCodegraphTooltip(status)} · stale`
    : "CodeGraph: status unavailable";
  return `${staleStatus} · ${formatRagStatusError(error)}`;
}

export const RagStatusIndicators: React.FC = () => {
  const config = useAppSelector(selectConfig);
  const enabled = hasUsableEngineEndpoint(config);
  const { data, error, isError, refetch } = useGetRagStatusQuery(undefined, {
    skip: !enabled,
    pollingInterval: 5000,
  });

  if (!enabled || (!data && !isError)) return null;

  const status = isError
    ? "error"
    : codegraphIndicatorState(data.codegraph_alive, data.codegraph);
  const tooltip = isError
    ? formatCodegraphErrorTooltip(data, error)
    : formatCodegraphTooltip(data);

  return (
    <div className={styles.ragStatuses} aria-label="Indexing status">
      <MiniStatusIndicator
        label="CodeGraph"
        status={status}
        tooltip={tooltip}
        onRefresh={() => void refetch()}
      />
    </div>
  );
};
