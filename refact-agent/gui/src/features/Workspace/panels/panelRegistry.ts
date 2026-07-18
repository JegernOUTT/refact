import type { ComponentType } from "react";

import type { PanelKind } from "../surfaceKey";
import { FilesPanel, GitPanel, TerminalPanel } from "./PanelPlaceholder";

export const PANEL_COMPONENTS: Record<PanelKind, ComponentType> = {
  files: FilesPanel,
  git: GitPanel,
  terminal: TerminalPanel,
};
