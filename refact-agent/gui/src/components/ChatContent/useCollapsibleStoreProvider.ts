import { useCallback, useMemo, useRef } from "react";
import type { CollapsibleStore } from "./CollapsibleStore";

export function useCollapsibleStoreProvider(
  resetKey: string,
): CollapsibleStore {
  const storeRef = useRef(new Map<string, boolean>());
  const prevKeyRef = useRef(resetKey);

  if (prevKeyRef.current !== resetKey) {
    storeRef.current = new Map();
    prevKeyRef.current = resetKey;
  }

  const get = useCallback((key: string): boolean | undefined => {
    return storeRef.current.get(key);
  }, []);

  const set = useCallback((key: string, open: boolean) => {
    storeRef.current.set(key, open);
  }, []);

  return useMemo(() => ({ get, set }), [get, set]);
}
