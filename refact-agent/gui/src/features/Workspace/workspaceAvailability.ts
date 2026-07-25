import type { Capabilities, Config } from "../Config/configSlice";

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
  host: Config["host"],
  capabilities: WorkspacePanelCapabilities,
  panelsForced: boolean,
): WorkspaceDockAvailability {
  const capabilitiesEnabled = host === "web";
  const files =
    panelsForced || (capabilitiesEnabled && capabilities.filesPanel);
  const git = panelsForced || (capabilitiesEnabled && capabilities.gitPanel);
  const dock = files || git;

  return {
    dock,
    files,
    git,
    tasks: dock,
    terminal:
      panelsForced || (capabilitiesEnabled && capabilities.terminalPanel),
  };
}
