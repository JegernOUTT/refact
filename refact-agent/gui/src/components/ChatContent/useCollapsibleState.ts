import { useCallback, useMemo, useState } from "react";

type CollapsibleState = Record<string, boolean>;

export function useCollapsibleState(defaultOpen = false) {
  const [state, setState] = useState<CollapsibleState>({});

  const isOpen = useCallback(
    (key: string) => (key in state ? state[key] : defaultOpen),
    [state, defaultOpen],
  );

  const setOpen = useCallback((key: string, open: boolean) => {
    setState((prev) => ({ ...prev, [key]: open }));
  }, []);

  const toggle = useCallback(
    (key: string) => {
      setState((prev) => {
        const current = key in prev ? prev[key] : defaultOpen;
        return { ...prev, [key]: !current };
      });
    },
    [defaultOpen],
  );

  const reset = useCallback(() => {
    setState({});
  }, []);

  return useMemo(
    () => ({ isOpen, setOpen, toggle, reset }),
    [isOpen, setOpen, toggle, reset],
  );
}

export type CollapsibleStateManager = ReturnType<typeof useCollapsibleState>;
