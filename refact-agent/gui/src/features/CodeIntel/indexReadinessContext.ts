import React from "react";

import type { CodeIntelIndexState } from "../../services/refact/types";

type ReadinessContextValue = {
  report: (key: string, state: CodeIntelIndexState | null) => void;
  notReadyState: CodeIntelIndexState | null;
};

export const CodeIntelReadinessContext =
  React.createContext<ReadinessContextValue | null>(null);
