import {
  useCallback,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import type { CollapsibleStore } from "../CollapsibleStore";
import { useCollapsibleStore } from "../useStoredOpen";

/** Choices the user made, kept per chat transcript next to its open/closed states. */
const remembered = new WeakMap<CollapsibleStore, Map<string, unknown>>();

/**
 * `useState` that survives the virtual list unmounting the row: with a key
 * and a transcript store, a value the user set is read back on mount.
 * Callers keep one value type per key.
 */
export function useRememberedState<T>(
  key: string | undefined,
  initial: T,
): [T, Dispatch<SetStateAction<T>>] {
  const store = useCollapsibleStore();
  const [value, setValue] = useState<T>(() => {
    const values = store && key ? remembered.get(store) : undefined;
    return key !== undefined && values?.has(key)
      ? (values.get(key) as T)
      : initial;
  });
  const update = useCallback<Dispatch<SetStateAction<T>>>(
    (action) =>
      setValue((current) => {
        const next =
          typeof action === "function"
            ? (action as (previous: T) => T)(current)
            : action;
        if (store && key) {
          let values = remembered.get(store);
          if (!values) {
            values = new Map();
            remembered.set(store, values);
          }
          values.set(key, next);
        }
        return next;
      }),
    [store, key],
  );
  return [value, update];
}
