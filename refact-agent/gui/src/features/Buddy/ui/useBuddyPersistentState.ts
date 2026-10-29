import { useCallback, useState, useSyncExternalStore } from "react";

type Listener = () => void;
type Updater<T> = T | ((previous: T) => T);

const values = new Map<string, unknown>();
const listeners = new Map<string, Set<Listener>>();

function notify(key: string) {
  listeners.get(key)?.forEach((listener) => listener());
}

function subscribe(key: string, listener: Listener): () => void {
  let set = listeners.get(key);
  if (!set) {
    set = new Set();
    listeners.set(key, set);
  }
  set.add(listener);
  const active = set;
  return () => {
    active.delete(listener);
    if (active.size === 0) listeners.delete(key);
  };
}

function resolve<T>(updater: Updater<T>, previous: T): T {
  return typeof updater === "function"
    ? (updater as (previous: T) => T)(previous)
    : updater;
}

export function readBuddyPersistentState<T>(key: string): T | undefined {
  return values.get(key) as T | undefined;
}

export function writeBuddyPersistentState<T>(key: string, value: T) {
  if (values.has(key) && Object.is(values.get(key), value)) return;
  values.set(key, value);
  notify(key);
}

export function resetBuddyPersistentState(key?: string) {
  if (key === undefined) {
    const keys = [...values.keys()];
    values.clear();
    keys.forEach(notify);
    return;
  }
  if (values.delete(key)) notify(key);
}

const noopSubscribe = () => () => undefined;

export function useBuddyPersistentState<T>(
  key: string | undefined,
  initial: T | (() => T),
): [T, (next: Updater<T>) => void] {
  const [initialValue] = useState(initial);
  const [localValue, setLocalValue] = useState<T>(initialValue);

  const subscribeKey = useCallback(
    (listener: Listener) =>
      key === undefined ? noopSubscribe() : subscribe(key, listener),
    [key],
  );
  const getSnapshot = useCallback(
    () =>
      key !== undefined && values.has(key)
        ? (values.get(key) as T)
        : initialValue,
    [key, initialValue],
  );
  const storedValue = useSyncExternalStore(
    subscribeKey,
    getSnapshot,
    getSnapshot,
  );

  const setValue = useCallback(
    (next: Updater<T>) => {
      if (key === undefined) {
        setLocalValue((previous) => resolve(next, previous));
        return;
      }
      const previous = values.has(key) ? (values.get(key) as T) : initialValue;
      writeBuddyPersistentState(key, resolve(next, previous));
    },
    [key, initialValue],
  );

  return [key === undefined ? localValue : storedValue, setValue];
}
