import { isAppRecord } from "../../services/refact/apps";
export type AppWorkspaceTab =
  | { kind: "chat"; ref: "self" }
  | { kind: "app"; binding_id: string };
export type AppWorkspaceLayout =
  | { kind: "leaf"; tabs: AppWorkspaceTab[]; active?: number }
  | {
      kind: "split";
      direction: "horizontal" | "vertical";
      sizes: number[];
      children: AppWorkspaceLayout[];
    };
export type AppWorkspaceBinding = {
  binding_id: string;
  app_id: string;
  params: Record<string, unknown>;
  state: Record<string, unknown>;
};
export type AppWorkspace = {
  schema_version: 1;
  revision: number;
  bindings: AppWorkspaceBinding[];
  layout: AppWorkspaceLayout;
};
export function isAppWorkspace(value: unknown): value is AppWorkspace {
  if (
    !isAppRecord(value) ||
    value.schema_version !== 1 ||
    !Number.isSafeInteger(value.revision) ||
    Number(value.revision) < 0 ||
    !Array.isArray(value.bindings) ||
    value.bindings.length > 32
  )
    return false;
  const uuid =
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const ids = new Set<string>();
  for (const binding of value.bindings as unknown[]) {
    if (
      !isAppRecord(binding) ||
      typeof binding.binding_id !== "string" ||
      !uuid.test(binding.binding_id) ||
      ids.has(binding.binding_id) ||
      typeof binding.app_id !== "string" ||
      !uuid.test(binding.app_id) ||
      !isAppRecord(binding.params) ||
      !isAppRecord(binding.state)
    )
      return false;
    ids.add(binding.binding_id);
  }
  let count = 0;
  let leaves = 0;
  let self = 0;
  const used = new Set<string>();
  const layout = (node: unknown, depth: number): boolean => {
    if (++count > 128 || depth > 6 || !isAppRecord(node)) return false;
    if (node.kind === "leaf") {
      if (++leaves > 32) return false;
      return (
        Array.isArray(node.tabs) &&
        node.tabs.length > 0 &&
        node.tabs.length <= 16 &&
        (node.active === undefined ||
          (Number.isInteger(node.active) &&
            Number(node.active) >= 0 &&
            Number(node.active) < node.tabs.length)) &&
        node.tabs.every((tab: unknown) => {
          if (!isAppRecord(tab)) return false;
          if (tab.kind === "chat" && tab.ref === "self") return ++self <= 1;
          if (
            tab.kind !== "app" ||
            typeof tab.binding_id !== "string" ||
            !ids.has(tab.binding_id) ||
            used.has(tab.binding_id)
          )
            return false;
          used.add(tab.binding_id);
          return true;
        })
      );
    }
    return (
      node.kind === "split" &&
      (node.direction === "horizontal" || node.direction === "vertical") &&
      Array.isArray(node.children) &&
      node.children.length >= 2 &&
      node.children.length <= 8 &&
      Array.isArray(node.sizes) &&
      node.sizes.length === node.children.length &&
      node.sizes.every(
        (size: unknown) =>
          typeof size === "number" && Number.isFinite(size) && size > 0,
      ) &&
      node.children.every((child: unknown) => layout(child, depth + 1))
    );
  };
  return layout(value.layout, 0);
}
