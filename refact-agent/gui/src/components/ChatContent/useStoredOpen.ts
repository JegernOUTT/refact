import {
  useCallback,
  useContext,
  useEffect,
  useState,
  createContext,
} from "react";
import type { CollapsibleStore } from "./CollapsibleStore";

const CollapsibleStoreContext = createContext<CollapsibleStore | null>(null);

export const CollapsibleStoreProvider = CollapsibleStoreContext.Provider;

export function useCollapsibleStore(): CollapsibleStore | null {
  return useContext(CollapsibleStoreContext);
}

export function useStoredOpen(
  storeKey: string | undefined,
  defaultOpen = false,
): [boolean, () => void, (open: boolean) => void] {
  const store = useCollapsibleStore();
  const [isOpen, setIsOpen] = useState(() => {
    if (storeKey && store) {
      const stored = store.get(storeKey);
      if (stored !== undefined) return stored;
    }
    return defaultOpen;
  });

  useEffect(() => {
    if (storeKey && store) store.set(storeKey, isOpen);
  }, [storeKey, store, isOpen]);

  const toggle = useCallback(() => setIsOpen((prev) => !prev), []);
  const setOpen = useCallback((open: boolean) => setIsOpen(open), []);

  return [isOpen, toggle, setOpen];
}
