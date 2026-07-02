import React from "react";

import { useAppSelector } from "../../hooks/useAppSelector";
import { selectConfig } from "../../features/Config/configSlice";
import { hasUsableEngineEndpoint } from "../../services/refact/apiUrl";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import type {
  CodeGraphState,
  CodeGraphStatus,
  RagStatus,
  VecDbStatus,
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

function vecdbIndicatorState(
  alive: string,
  status: VecDbStatus | null,
): IndicatorState {
  if (alive === "turned_off") return "idle";
  if (alive !== "working") return "error";
  if (!status) return "success";
  if (status.state === "done") return "success";
  if (status.state === "cooldown") return "warning";
  return "working";
}

function formatCodegraphTooltip(status: RagStatus): string {
  const codegraph = status.codegraph;
  const alive = status.codegraph_alive;
  if (!codegraph) return `Codegraph: ${alive}`;
  const suffix = codegraph.error ? ` · ${codegraph.error}` : "";
  return `Codegraph: ${alive} · ${codegraph.counts.files} files · ${codegraph.counts.nodes} nodes · ${codegraph.queued} queued${suffix}`;
}

function formatVecdbTooltip(status: RagStatus): string {
  const vecdb = status.vecdb;
  const alive = status.vecdb_alive;
  if (!vecdb) {
    const suffix = status.vec_db_error ? ` · ${status.vec_db_error}` : "";
    return `VecDB: ${alive}${suffix}`;
  }
  return `VecDB: ${alive} · ${vecdb.state} · ${vecdb.files_unprocessed}/${vecdb.files_total} files`;
}

function formatAstTooltip(status: RagStatus): string {
  return `AST: ${status.ast_alive}`;
}

export const RagStatusIndicators: React.FC = () => {
  const config = useAppSelector(selectConfig);
  const enabled = hasUsableEngineEndpoint(config);
  const { data, refetch } = useGetRagStatusQuery(undefined, {
    skip: !enabled,
    pollingInterval: 5000,
  });

  if (!enabled || !data) return null;

  return (
    <div className={styles.ragStatuses} aria-label="Indexing status">
      <MiniStatusIndicator
        label="Codegraph"
        status={codegraphIndicatorState(data.codegraph_alive, data.codegraph)}
        tooltip={formatCodegraphTooltip(data)}
        onRefresh={() => void refetch()}
      />
      <MiniStatusIndicator
        label="VecDB"
        status={vecdbIndicatorState(data.vecdb_alive, data.vecdb)}
        tooltip={formatVecdbTooltip(data)}
        onRefresh={() => void refetch()}
      />
      <MiniStatusIndicator
        label="AST"
        status={stateFromAlive(data.ast_alive)}
        tooltip={formatAstTooltip(data)}
        onRefresh={() => void refetch()}
      />
    </div>
  );
};
