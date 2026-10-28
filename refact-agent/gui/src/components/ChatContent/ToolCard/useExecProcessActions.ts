import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useAppSelector } from "../../../hooks/useAppSelector";
import {
  selectCapabilities,
  selectConfig,
} from "../../../features/Config/configSlice";
import {
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
} from "../../../services/refact/apiUrl";
import { killExec, readExec } from "../../../services/refact/exec";
import type { ToolEnrichmentReference } from "../../../services/refact/types";
import { ideOpenFile } from "../../../hooks/useEventBusForIDE";
import { usePostMessage } from "../../../hooks/usePostMessage";

type ActionState = {
  source: object;
  loading?: boolean;
  stopping?: boolean;
  content?: string;
  error?: string;
};

function errorText(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause);
}

export function useExecProcessActions(
  processId: string | undefined,
  threadId: string,
  toolCallId: string | undefined,
  logPath: string | null,
) {
  const config = useAppSelector(selectConfig);
  const capabilities = useAppSelector(selectCapabilities);
  const postMessage = usePostMessage();
  const host = config.host;
  const endpointIdentity = getEngineEndpointIdentity(config);
  const canReadTranscript = Boolean(
    processId && threadId && hasUsableEngineEndpoint(config),
  );
  const canOpenInIde =
    capabilities.openFileInIde &&
    ((host === "vscode" && typeof window.acquireVsCodeApi === "function") ||
      (host === "jetbrains" &&
        typeof window.postIntellijMessage === "function") ||
      (host === "ide" &&
        (typeof window.acquireVsCodeApi === "function" ||
          typeof window.postIntellijMessage === "function")));
  const source = useMemo(
    () => ({
      endpointIdentity,
      threadId,
      toolCallId,
      processId,
      logPath,
      apiKey: config.apiKey,
      canReadTranscript,
      canOpenInIde,
    }),
    [
      endpointIdentity,
      threadId,
      toolCallId,
      processId,
      logPath,
      config.apiKey,
      canReadTranscript,
      canOpenInIde,
    ],
  );
  const sourceRef = useRef<object | null>(source);
  sourceRef.current = source;
  const [state, setState] = useState<ActionState | null>(null);
  const current = state?.source === source ? state : null;
  useEffect(() => {
    sourceRef.current = source;
    return () => {
      sourceRef.current = null;
    };
  }, [source]);

  const openLog = useCallback(() => {
    if (!logPath || !canOpenInIde) return;
    try {
      postMessage(ideOpenFile({ file_path: logPath }));
    } catch (cause) {
      setState({ source, error: errorText(cause) });
    }
  }, [canOpenInIde, logPath, postMessage, source]);

  const openPath = useCallback(
    (reference: ToolEnrichmentReference) => {
      if (!canOpenInIde) return;
      postMessage(
        ideOpenFile({
          file_path: reference.target,
          line: reference.line1 ?? undefined,
        }),
      );
    },
    [canOpenInIde, postMessage],
  );

  const readTranscript = useCallback(async () => {
    if (!canReadTranscript || !processId || current?.loading) return;
    setState({ source, loading: true });
    try {
      const response = await readExec(
        processId,
        0,
        config,
        threadId,
        config.apiKey ?? undefined,
      );
      if (sourceRef.current !== source) return;
      setState({
        source,
        content: response.chunks.map((chunk) => chunk.text).join(""),
      });
    } catch (cause) {
      if (sourceRef.current !== source) return;
      setState({ source, error: errorText(cause) });
    }
  }, [
    canReadTranscript,
    config,
    current?.loading,
    processId,
    source,
    threadId,
  ]);

  const stop = useCallback(async () => {
    if (!canReadTranscript || !processId || current?.stopping) return;
    setState({ source, stopping: true });
    try {
      await killExec(processId, config, threadId, config.apiKey ?? undefined);
      if (sourceRef.current === source) setState(null);
    } catch (cause) {
      if (sourceRef.current !== source) return;
      setState({ source, error: errorText(cause) });
    }
  }, [
    canReadTranscript,
    config,
    current?.stopping,
    processId,
    source,
    threadId,
  ]);

  return {
    canOpenInIde,
    canReadTranscript,
    state: current,
    openLog,
    openPath,
    readTranscript,
    stop,
  };
}
