import React from "react";

import type { CodeIntelIndexState } from "../../services/refact/types";
import { CodeIntelReadinessContext } from "./indexReadinessContext";

export function CodeIntelReadinessProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [states, setStates] = React.useState<
    Record<string, CodeIntelIndexState>
  >({});
  const report = React.useCallback(
    (key: string, state: CodeIntelIndexState | null) => {
      setStates((previous) => {
        if (!state) {
          if (!(key in previous)) return previous;
          return Object.fromEntries(
            Object.entries(previous).filter(([entryKey]) => entryKey !== key),
          );
        }
        if (previous[key] === state) return previous;
        return { ...previous, [key]: state };
      });
    },
    [],
  );
  const notReadyState = React.useMemo(() => {
    const notReady = Object.values(states).filter(
      (state) => !state.cross_file_ready,
    );
    if (notReady.length === 0) return null;
    return notReady.reduce((max, state) =>
      state.queued > max.queued ? state : max,
    );
  }, [states]);

  const value = React.useMemo(
    () => ({ report, notReadyState }),
    [notReadyState, report],
  );

  return (
    <CodeIntelReadinessContext.Provider value={value}>
      {children}
    </CodeIntelReadinessContext.Provider>
  );
}
