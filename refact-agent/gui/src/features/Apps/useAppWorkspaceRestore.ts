export { appLayoutToPane } from "./appLayout";

/** Persistence and restoration are store-owned, including while UI is unmounted. */
export function useAppWorkspaceRestore() {
  // Kept as a compatibility hook for existing page call sites.
  return undefined;
}
