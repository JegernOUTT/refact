import React, { useCallback, useMemo } from "react";
import classNames from "classnames";
import { Server } from "lucide-react";

import { push } from "../../features/Pages/pagesSlice";
import { selectConfig } from "../../features/Config/configSlice";
import {
  selectBackendStatus,
  selectCurrentChatSseStatus,
  selectIsFullyConnected,
} from "../../features/Connection";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { useOpenUrl } from "../../hooks/useOpenUrl";
import { hasUsableEngineEndpoint } from "../../services/refact/apiUrl";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import type { CodeGraphState } from "../../services/refact/types";
import { Button, Popover, StatusDot, Tooltip } from "../ui";
import type { StatusDotProps } from "../ui";
import { ConnectionStatusIndicator } from "./ConnectionStatusIndicator";
import { RagStatusIndicators } from "./RagStatusIndicators";
import { formatEngineHost, resolveBrowserEngineUrl } from "./engineUrl";
import styles from "./EngineStatusChip.module.css";

const RAG_POLLING_INTERVAL_MS = 5000;

type DotStatus = NonNullable<StatusDotProps["status"]>;

function connectionDotStatus(
  isConnected: boolean,
  isReconnecting: boolean,
): DotStatus {
  if (isConnected) return "success";
  if (isReconnecting) return "warning";
  return "error";
}

function indexDotStatusFromAlive(alive: string): DotStatus {
  if (alive === "working") return "success";
  if (alive === "indexing") return "running";
  if (alive === "turned_off" || alive === "") return "idle";
  return "error";
}

function indexDotStatus(
  alive: string | undefined,
  state: CodeGraphState | null,
): DotStatus {
  if (state === "indexing") return "running";
  if (state === "working") return "success";
  if (state === "turned_off") return "idle";
  if (state === "error") return "error";
  return indexDotStatusFromAlive(alive ?? "");
}

export const EngineStatusChip: React.FC = () => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const openUrl = useOpenUrl();
  const visible = useIsDocumentVisible();
  const isConnected = useAppSelector(selectIsFullyConnected);
  const backendStatus = useAppSelector(selectBackendStatus);
  const sseStatus = useAppSelector(selectCurrentChatSseStatus);
  const ragEnabled = hasUsableEngineEndpoint(config);
  const { data, isError } = useGetRagStatusQuery(undefined, {
    skip: !ragEnabled,
    pollingInterval: visible ? RAG_POLLING_INTERVAL_MS : 0,
  });

  const engineUrl = useMemo(() => resolveBrowserEngineUrl(config), [config]);
  const engineHost = useMemo(() => formatEngineHost(engineUrl), [engineUrl]);
  const isReconnecting =
    sseStatus === "connecting" || backendStatus === "unknown";
  const connectionStatus = connectionDotStatus(isConnected, isReconnecting);
  const indexStatus = isError
    ? "error"
    : indexDotStatus(data?.codegraph_alive, data?.codegraph?.state ?? null);

  const onOpenEngineUrl = useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>) => {
      event.preventDefault();
      openUrl(engineUrl);
    },
    [engineUrl, openUrl],
  );

  const onOpenRefactDaemon = useCallback(() => {
    dispatch(push({ name: "refact daemon" }));
  }, [dispatch]);

  return (
    <Popover>
      <Tooltip>
        <Tooltip.Trigger asChild>
          <Popover.Trigger asChild>
            <button
              type="button"
              aria-label="Engine status"
              className={classNames(styles.chip, "rf-pressable")}
            >
              <StatusDot status={connectionStatus} size="small" />
              <StatusDot status={indexStatus} size="small" />
              <span className={styles.chipLabel}>{engineHost}</span>
            </button>
          </Popover.Trigger>
        </Tooltip.Trigger>
        <Tooltip.Content side="bottom">Engine status</Tooltip.Content>
      </Tooltip>

      <Popover.Content align="end" scrollable={false}>
        <div className={styles.panel}>
          <div className={styles.row}>
            <span className={styles.rowLabel}>Connection</span>
            <ConnectionStatusIndicator />
          </div>
          <div className={styles.row}>
            <span className={styles.rowLabel}>Indexing</span>
            <RagStatusIndicators />
          </div>
          <a
            className={styles.engineUrl}
            href={engineUrl}
            title={engineUrl}
            aria-label={`Engine URL ${engineUrl}`}
            onClick={onOpenEngineUrl}
          >
            {engineUrl}
          </a>
          <Button
            aria-label="Refact Daemon"
            leftIcon={Server}
            onClick={onOpenRefactDaemon}
            size="sm"
            variant="soft"
          >
            Open Daemon
          </Button>
        </div>
      </Popover.Content>
    </Popover>
  );
};
