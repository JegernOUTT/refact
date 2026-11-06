import type { Middleware } from "@reduxjs/toolkit";
import { collectTabIds } from "../ChatPanes/panesTree";
import { useCallback, useSyncExternalStore, type SetStateAction } from "react";
import { useStore } from "react-redux";
import type { RootState } from "../../app/store";
import { useAppSelector } from "../../hooks";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";

type Slot = { value: unknown; listeners: Set<() => void> };
type Registry = Map<string, Map<string, Slot>>;
const sessionKey = (store: { getState: unknown }) =>
  store.getState as () => RootState;
const sessions = new WeakMap<() => RootState, Registry>();

export const surfaceSessionScope = (state: RootState) =>
  JSON.stringify([
    getEngineEndpointIdentity(state.config),
    state.current_project.projectRoot,
    state.current_project.workspaceRoots,
    state.current_project.name,
  ]);

export const surfaceSessionMiddleware: Middleware<object, RootState> =
  (api) => (next) => (action) => {
    const original = api.getState();
    const result = next(action);
    const state = api.getState();
    if (
      (original.config !== state.config ||
        original.current_project !== state.current_project) &&
      surfaceSessionScope(original) !== surfaceSessionScope(state)
    ) {
      sessions.delete(sessionKey(api));
    } else if (original.workspace !== state.workspace) {
      const paths = (workspace: RootState["workspace"]) =>
        new Set([
          ...workspace.tabs,
          ...Object.values(workspace.groups).flatMap((group) =>
            group ? collectTabIds(group.root) : [],
          ),
        ]);
      const remaining = paths(state.workspace);
      const registry = sessions.get(sessionKey(api));
      for (const key of paths(original.workspace)) {
        if (key.startsWith("file:") && !remaining.has(key))
          registry?.delete(key.slice(5));
      }
    }
    return result;
  };

export function useSurfaceSessionState<T>(
  path: string,
  field: string,
  initialValue: T,
  version = "",
): [T, (value: SetStateAction<T>) => void] {
  const store = useStore<RootState>();
  const scope = useAppSelector(surfaceSessionScope);
  let registry = sessions.get(sessionKey(store));
  if (!registry) {
    registry = new Map();
    sessions.set(sessionKey(store), registry);
  }
  let fields = registry.get(path);
  if (!fields) {
    fields = new Map();
    registry.set(path, fields);
  }
  const key = JSON.stringify([scope, field]);
  const versionKey = `${key}:version`;
  if (fields.get(versionKey)?.value !== version) {
    fields.set(versionKey, { value: version, listeners: new Set() });
    fields.delete(key);
  }
  let slot = fields.get(key);
  if (!slot) {
    slot = { value: initialValue, listeners: new Set() };
    fields.set(key, slot);
  }
  const current = slot;
  const subscribe = useCallback(
    (listener: () => void) => {
      current.listeners.add(listener);
      return () => {
        current.listeners.delete(listener);
      };
    },
    [current],
  );
  const snapshot = useCallback(() => current.value as T, [current]);
  const value = useSyncExternalStore(subscribe, snapshot, snapshot);
  const setValue = useCallback(
    (next: SetStateAction<T>) => {
      current.value =
        typeof next === "function"
          ? (next as (previous: T) => T)(current.value as T)
          : next;
      current.listeners.forEach((listener) => listener());
    },
    [current],
  );
  return [value, setValue];
}
