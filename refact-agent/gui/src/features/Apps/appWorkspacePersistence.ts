import { createAction, type Middleware } from "@reduxjs/toolkit";
import type { RootState, AppDispatch } from "../../app/store";
import {
  buildApiUrl,
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
} from "../../services/refact/apiUrl";
import { collectTabIds, findLeafByTab } from "../ChatPanes/panesTree";
import { change } from "../Pages/pagesSlice";
import {
  closeTab,
  openTab,
  restoreAppWorkspace,
  selectWorkspaceFocusedChatId,
  selectFocusedWorkspaceSurfaceKey,
  setActiveTab,
  splitPaneWithSurface,
} from "../Workspace/workspaceSlice";
import {
  appLayoutToPane,
  canonicalAppLayout,
  paneToAppLayout,
} from "./appLayout";
import {
  appBindingPending,
  appViewOpened,
  openRailApp,
  retryAppWorkspaceSave,
} from "./appsSlice";
import { isAppWorkspace, type AppWorkspace } from "./appWorkspace";
import { v4 as uuidv4 } from "uuid";

export type WorkspaceSaveStatus = "saving" | "saved" | "error" | "readonly";
export const openRequestedApp = createAction<{
  appId: string;
  name: string;
  chatId?: string;
  bindingId?: string;
}>("apps/openRequestedApp");
export const workspaceSaveStatus = (
  chatId: string,
  status: WorkspaceSaveStatus,
) => ({ type: "apps/workspaceSaveStatus", payload: { chatId, status } });
type Entry = {
  desired: AppWorkspace;
  acknowledged: AppWorkspace;
  inFlight: boolean;
  localGeneration: number;
  sentGeneration?: number;
  sentWorkspace?: AppWorkspace;
  dirty: boolean;
  needsRefresh?: boolean;
  restored: boolean;
  observedLayout?: string;
  timer?: ReturnType<typeof setTimeout>;
};
const fingerprint = (workspace: AppWorkspace) =>
  JSON.stringify(
    { ...workspace, revision: 0, layout: canonicalAppLayout(workspace.layout) },
    (_key, value: unknown) =>
      typeof value === "object" && value !== null && !Array.isArray(value)
        ? Object.fromEntries(
            Object.entries(value).sort(([left], [right]) =>
              left.localeCompare(right),
            ),
          )
        : value,
  );

/** One latest-desired queue per store, endpoint and trajectory. Acks never restore UI. */
export const appWorkspacePersistenceMiddleware: Middleware = (api) => {
  const dispatch = api.dispatch as AppDispatch;
  const getState = () => api.getState() as RootState;
  let scope = "";
  let entries = new Map<string, Entry>();
  let restoring = false;
  const pendingOpens = new Map<
    string,
    ReturnType<typeof openRailApp> | ReturnType<typeof openRequestedApp>
  >();
  const reset = () => {
    const nextScope = getEngineEndpointIdentity(getState().config);
    if (nextScope === scope) return;
    for (const entry of entries.values())
      if (entry.timer) clearTimeout(entry.timer);
    scope = nextScope;
    entries = new Map();
    pendingOpens.clear();
  };
  const reconcile = (
    chatId: string,
    entry: Entry,
    remote: AppWorkspace,
    hasLocalEdits: boolean,
  ) => {
    const base = entry.sentWorkspace ?? entry.acknowledged;
    entry.acknowledged = remote;
    if (!hasLocalEdits) {
      if (fingerprint(entry.desired) !== fingerprint(remote))
        entry.restored = false;
      entry.desired = remote;
      entry.dirty = false;
    } else {
      // Remote records own updates and deletions; only genuinely new local
      // bindings absent from the save/base may be carried forward.
      const bindings = [
        ...remote.bindings,
        ...entry.desired.bindings.filter(
          (binding) =>
            !base.bindings.some(
              (other) => other.binding_id === binding.binding_id,
            ) &&
            !remote.bindings.some(
              (other) => other.binding_id === binding.binding_id,
            ),
        ),
      ];
      const layout =
        paneToAppLayout(
          appLayoutToPane(entry.desired.layout, chatId),
          chatId,
          new Set(bindings.map((binding) => binding.binding_id)),
        ) ?? remote.layout;
      const desired = { ...entry.desired, bindings, layout };
      if (fingerprint(desired) !== fingerprint(entry.desired))
        entry.restored = false;
      entry.desired = desired;
    }
    entry.dirty = fingerprint(entry.desired) !== fingerprint(remote);
    if (!entry.dirty) {
      entry.needsRefresh = false;
      if (!entry.inFlight && getState().apps.saveStatus?.[chatId] === "error")
        dispatch(workspaceSaveStatus(chatId, "saved"));
    }
    for (const binding of remote.bindings)
      if (getState().apps.pendingBindings?.[binding.binding_id])
        dispatch(
          appBindingPending({ bindingId: binding.binding_id, pending: false }),
        );
  };
  const entryFor = (chatId: string): Entry | undefined => {
    const thread = getState().chat.threads[chatId]?.thread;
    if (
      thread?.app_workspace_raw != null &&
      !isAppWorkspace(thread.app_workspace_raw)
    )
      return undefined;
    let entry = entries.get(chatId);
    const remote = thread?.app_workspace;
    if (!entry && remote) {
      entry = {
        desired: remote,
        acknowledged: remote,
        dirty: false,
        inFlight: false,
        localGeneration: 0,
        restored: false,
      };
      entries.set(chatId, entry);
    }
    if (entry && remote && remote.revision > entry.acknowledged.revision)
      reconcile(
        chatId,
        entry,
        remote,
        entry.inFlight
          ? entry.localGeneration !== entry.sentGeneration
          : entry.dirty,
      );
    return entry;
  };
  const clearTimer = (entry: Entry) => {
    if (entry.timer) clearTimeout(entry.timer);
    entry.timer = undefined;
  };
  const save = async (chatId: string, entry: Entry) => {
    clearTimer(entry);
    if (
      entry.inFlight ||
      (entry.needsRefresh ?? false) ||
      !entry.dirty ||
      !hasUsableEngineEndpoint(getState().config)
    )
      return;
    entry.inFlight = true;
    const endpoint = scope;
    const config = getState().config;
    dispatch(workspaceSaveStatus(chatId, "saving"));
    let failed = false;
    try {
      const sent = {
        ...entry.desired,
        revision: entry.acknowledged.revision,
      };
      entry.sentGeneration = entry.localGeneration;
      entry.sentWorkspace = sent;
      const response = await fetch(
        buildApiUrl(config, `/v1/chats/${encodeURIComponent(chatId)}/commands`),
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            ...(config.apiKey
              ? { Authorization: `Bearer ${config.apiKey}` }
              : {}),
          },
          body: JSON.stringify({
            type: "set_app_workspace",
            expected_revision: sent.revision,
            workspace: sent,
            client_request_id: uuidv4(),
          }),
        },
      );
      if (scope !== endpoint || entries.get(chatId) !== entry) return;
      const result: unknown = await response.json();
      if (scope !== endpoint || entries.get(chatId) !== entry) return;
      const workspace =
        typeof result === "object" &&
        result !== null &&
        "app_workspace" in result
          ? result.app_workspace
          : undefined;
      if (!response.ok || !isAppWorkspace(workspace))
        throw new Error("Workspace save failed");
      if (workspace.revision >= entry.acknowledged.revision)
        entry.acknowledged = workspace;
      entry.dirty =
        entry.localGeneration !== entry.sentGeneration &&
        fingerprint(entry.desired) !== fingerprint(entry.acknowledged);
      if (!entry.dirty) {
        entry.desired = entry.acknowledged;
        if (fingerprint(workspace) !== fingerprint(entry.acknowledged))
          entry.restored = false;
      }
      for (const binding of workspace.bindings)
        dispatch(
          appBindingPending({
            bindingId: binding.binding_id,
            pending: false,
          }),
        );
    } catch {
      failed = entry.dirty;
      entry.needsRefresh = failed;
    } finally {
      clearTimer(entry);
      entry.inFlight = false;
      entry.sentWorkspace = undefined;
      entry.sentGeneration = undefined;
      if (scope === endpoint && entries.get(chatId) === entry) {
        dispatch(workspaceSaveStatus(chatId, failed ? "error" : "saved"));
        if (entry.dirty && !failed) void save(chatId, entry);
      }
    }
  };
  const retry = async (chatId: string, entry: Entry) => {
    if (entry.inFlight || !hasUsableEngineEndpoint(getState().config)) return;
    clearTimer(entry);
    if (!entry.dirty) {
      entry.needsRefresh = false;
      dispatch(workspaceSaveStatus(chatId, "saved"));
      return;
    }
    entry.inFlight = true;
    const endpoint = scope;
    const config = getState().config;
    dispatch(workspaceSaveStatus(chatId, "saving"));
    let refreshed = false;
    try {
      const response = await fetch(
        buildApiUrl(config, `/v1/trajectories/${encodeURIComponent(chatId)}`),
        {
          method: "GET",
          cache: "no-store",
          headers: config.apiKey
            ? { Authorization: `Bearer ${config.apiKey}` }
            : {},
        },
      );
      const result: unknown = await response.json();
      if (scope !== endpoint || entries.get(chatId) !== entry) return;
      const rawWorkspace =
        typeof result === "object" &&
        result !== null &&
        "app_workspace" in result
          ? result.app_workspace
          : undefined;
      const workspace =
        rawWorkspace == null &&
        typeof result === "object" &&
        result !== null &&
        "id" in result &&
        result.id === chatId &&
        entry.acknowledged.revision === 0
          ? entry.acknowledged
          : rawWorkspace;
      if (!response.ok || !isAppWorkspace(workspace))
        throw new Error("Workspace refresh failed");
      if (workspace.revision >= entry.acknowledged.revision)
        reconcile(chatId, entry, workspace, entry.dirty);
      entry.dirty =
        fingerprint(entry.desired) !== fingerprint(entry.acknowledged);
      entry.needsRefresh = false;
      refreshed = true;
      if (!entry.restored && getState().chat.open_thread_ids.includes(chatId))
        restore(chatId, entry);
    } catch {
      entry.needsRefresh = entry.dirty;
    } finally {
      entry.inFlight = false;
      if (scope === endpoint && entries.get(chatId) === entry) {
        if (refreshed && entry.dirty) void save(chatId, entry);
        else
          dispatch(
            workspaceSaveStatus(chatId, entry.dirty ? "error" : "saved"),
          );
      }
    }
  };
  const capture = (state: RootState) => {
    for (const chatId of state.chat.open_thread_ids) {
      const entry = entryFor(chatId);
      if (!entry?.restored) continue;
      const group = Object.values(state.workspace.groups).find(
        (group) =>
          group && collectTabIds(group.root).includes(`chat:${chatId}`),
      );
      if (!group) continue;
      const layout = paneToAppLayout(
        group.root,
        chatId,
        new Set(entry.desired.bindings.map((binding) => binding.binding_id)),
      );
      if (!layout) continue;
      const observed = JSON.stringify(canonicalAppLayout(layout));
      if (entry.observedLayout === observed) continue;
      entry.observedLayout = observed;
      const desired = { ...entry.desired, layout };
      if (
        !isAppWorkspace(desired) ||
        fingerprint(desired) === fingerprint(entry.desired)
      )
        continue;
      entry.desired = desired;
      entry.localGeneration++;
      entry.dirty = true;
      if (entry.timer) clearTimeout(entry.timer);
      entry.timer = setTimeout(() => void save(chatId, entry), 100);
    }
  };
  const restore = (chatId: string, entry: Entry) => {
    restoring = true;
    try {
      for (const binding of entry.desired.bindings)
        dispatch(
          appViewOpened({
            key: `app:${binding.binding_id}`,
            view: {
              appId: binding.app_id,
              bindingId: binding.binding_id,
              chatId,
              name: binding.app_id,
            },
          }),
        );
      const root = appLayoutToPane(entry.desired.layout, chatId);
      let leaf = root;
      while (leaf.kind === "split") leaf = leaf.children[0];
      dispatch(restoreAppWorkspace({ chatId, root, focusedLeafId: leaf.id }));
      entry.observedLayout = JSON.stringify(
        canonicalAppLayout(entry.desired.layout),
      );
      entry.restored = true;
    } finally {
      restoring = false;
    }
  };
  return (next) => (action) => {
    reset();
    const before = getState();
    if (!restoring) capture(before);
    if (
      typeof action === "object" &&
      action !== null &&
      "type" in action &&
      action.type === "chatThread/switchToThread"
    ) {
      for (const [chatId, entry] of entries) void save(chatId, entry);
    }
    const result = next(action);
    reset();
    if (restoring) return result;
    const state = getState();
    for (const chatId of before.chat.open_thread_ids) {
      if (state.chat.open_thread_ids.includes(chatId)) continue;
      const entry = entries.get(chatId);
      if (entry) {
        entry.restored = false;
        void save(chatId, entry);
      }
      for (const [key, view] of Object.entries(state.apps.views))
        if (view?.chatId === chatId) dispatch(closeTab(key));
    }
    for (const chatId of state.chat.open_thread_ids) {
      const entry = entryFor(chatId);
      if (
        entry &&
        !entry.restored &&
        state.chat.threads[chatId]?.snapshot_received
      )
        restore(chatId, entry);
    }
    if (openRailApp.match(action) || openRequestedApp.match(action)) {
      const requested = openRequestedApp.match(action)
        ? action.payload
        : undefined;
      const surface = selectFocusedWorkspaceSurfaceKey(state);
      const focused =
        selectWorkspaceFocusedChatId(state) ??
        (surface ? state.apps.views[surface]?.chatId : undefined);
      const owner = requested ? requested.chatId : focused;
      const chatId =
        owner && state.chat.open_thread_ids.includes(owner) ? owner : null;
      // A bound tool request never becomes a standalone or another chat's view.
      if (requested?.chatId && !chatId) return result;
      let key = `app:standalone:${action.payload.appId}`;
      if (chatId) {
        if (!state.chat.threads[chatId]?.snapshot_received) {
          pendingOpens.set(`${chatId}:${action.payload.appId}`, action);
          return result;
        }
        const thread = state.chat.threads[chatId].thread;
        if (
          thread.app_workspace_raw != null &&
          !isAppWorkspace(thread.app_workspace_raw)
        ) {
          dispatch(workspaceSaveStatus(chatId, "readonly"));
          return result;
        }
        let entry = entryFor(chatId);
        if (!entry) {
          const workspace: AppWorkspace = {
            schema_version: 1,
            revision: 0,
            bindings: [],
            layout: { kind: "leaf", tabs: [{ kind: "chat", ref: "self" }] },
          };
          entry = {
            desired: workspace,
            acknowledged: workspace,
            inFlight: false,
            localGeneration: 0,
            dirty: false,
            restored: true,
          };
          entries.set(chatId, entry);
        }
        entry.restored = true;
        let binding = entry.desired.bindings.find(
          (binding) =>
            binding.app_id === action.payload.appId &&
            (!requested?.bindingId ||
              binding.binding_id === requested.bindingId),
        );
        if (requested?.bindingId && !binding) {
          pendingOpens.set(`${chatId}:${action.payload.appId}`, action);
          return result;
        }
        if (!binding) {
          binding = {
            binding_id: uuidv4(),
            app_id: action.payload.appId,
            params: {},
            state: {},
          };
          entry.desired = {
            ...entry.desired,
            bindings: [...entry.desired.bindings, binding],
          };
          entry.localGeneration++;
          entry.dirty = true;
          dispatch(
            appBindingPending({ bindingId: binding.binding_id, pending: true }),
          );
        }
        key = `app:${binding.binding_id}`;
        dispatch(
          appViewOpened({
            key,
            view: { ...action.payload, bindingId: binding.binding_id, chatId },
          }),
        );
        const current = getState();
        const owner = Object.entries(current.workspace.groups).find(
          ([, group]) =>
            group && collectTabIds(group.root).includes(`chat:${chatId}`),
        );
        if (owner?.[1]) {
          const [tabId, group] = owner;
          if (findLeafByTab(group.root, key)) dispatch(setActiveTab(key));
          else {
            const leaf = findLeafByTab(group.root, `chat:${chatId}`);
            if (leaf)
              dispatch(
                splitPaneWithSurface({
                  tabId,
                  leafId: leaf.id,
                  surfaceKey: key,
                  dir: "row",
                  placement: "after",
                }),
              );
          }
        }
        capture(getState());
        void save(chatId, entry);
      } else {
        dispatch(appViewOpened({ key, view: action.payload }));
        dispatch(openTab(key));
      }
      dispatch(change({ name: "chat" }));
    } else if (retryAppWorkspaceSave.match(action)) {
      const entry = entries.get(action.payload);
      if (entry) void retry(action.payload, entry);
    } else if (
      typeof action === "object" &&
      action !== null &&
      "type" in action &&
      typeof action.type === "string" &&
      action.type.startsWith("workspace/") &&
      action.type !== "workspace/restoreAppWorkspace" &&
      action.type !== "workspace/reconcileWorkspace"
    )
      capture(getState());
    // Replay only against the original still-focused chat after its authoritative
    // snapshot. Never reinterpret a delayed click as a new chat/endpoint intent.
    for (const [intentKey, intent] of pendingOpens) {
      const chatId = intentKey.slice(0, intentKey.lastIndexOf(":"));
      const current = getState();
      const surface = selectFocusedWorkspaceSurfaceKey(current);
      const focused =
        selectWorkspaceFocusedChatId(current) ??
        (surface ? current.apps.views[surface]?.chatId : undefined);
      if (
        !current.chat.open_thread_ids.includes(chatId) ||
        (!openRequestedApp.match(intent) && focused !== chatId)
      ) {
        pendingOpens.delete(intentKey);
      } else if (
        current.chat.threads[chatId]?.snapshot_received &&
        (!openRequestedApp.match(intent) ||
          !intent.payload.bindingId ||
          entryFor(chatId)?.desired.bindings.some(
            (binding) => binding.binding_id === intent.payload.bindingId,
          ))
      ) {
        pendingOpens.delete(intentKey);
        dispatch(intent);
      }
    }
    return result;
  };
};
