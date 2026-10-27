import { useId, useMemo, useState } from "react";
import { useAppSelector } from "../../hooks";
import { useConfig } from "../../hooks/useConfig";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import { selectCurrentThreadId, selectModel } from "../Chat/Thread/selectors";

export function usePanelScope() {
  const config = useConfig();
  const chatId = useAppSelector(selectCurrentThreadId);
  const model = useAppSelector(selectModel);
  const id = useId();
  const endpoint = getEngineEndpointIdentity(config);
  const source = useMemo(
    () => ({
      endpoint,
      token: config.apiKey,
      project: config.currentWorkspaceName,
      ready: config.backendReady,
      connection: config.connectionStatus,
      chatId,
      model,
    }),
    [
      endpoint,
      config.apiKey,
      config.currentWorkspaceName,
      config.backendReady,
      config.connectionStatus,
      chatId,
      model,
    ],
  );
  const [scope, setScope] = useState({ source, generation: 0 });
  const generation =
    scope.source === source ? scope.generation : scope.generation + 1;
  if (scope.source !== source) setScope({ source, generation });
  return `${endpoint}:${id}:${generation}`;
}
