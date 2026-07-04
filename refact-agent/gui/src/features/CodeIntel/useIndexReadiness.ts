import React from "react";

import type { CodeIntelIndexState } from "../../services/refact/types";
import { CodeIntelReadinessContext } from "./indexReadinessContext";
import { indexStateFromResponse } from "./indexReadinessState";

export function useReportIndexReadiness(key: string, response: unknown): void {
  const context = React.useContext(CodeIntelReadinessContext);
  const state = React.useMemo(
    () => indexStateFromResponse(response),
    [response],
  );

  React.useEffect(() => {
    context?.report(key, state);
  }, [context, key, state]);
}

export function useCodeIntelReadinessState(): CodeIntelIndexState | null {
  return React.useContext(CodeIntelReadinessContext)?.notReadyState ?? null;
}
