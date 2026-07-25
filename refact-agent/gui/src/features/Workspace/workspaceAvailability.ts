import type { Capabilities } from "../Config/configSlice";

type WorkspacePanelCapabilities = Pick<
  Capabilities,
  "filesPanel" | "gitPanel" | "terminalPanel"
>;

export type WorkspaceDockAvailability = {
  dock: boolean;
  files: boolean;
  git: boolean;
  tasks: boolean;
  terminal: boolean;
};

export function resolveWorkspaceDockAvailability(
  capabilities: WorkspacePanelCapabilities,
  panelsForced: boolean,
): WorkspaceDockAvailability {
  const files = capabilities.filesPanel || panelsForced;
  const git = capabilities.gitPanel || panelsForced;
  const dock = files || git;

  return {
    dock,
    files,
    git,
    tasks: dock,
    terminal: capabilities.terminalPanel || panelsForced,
  };
}
