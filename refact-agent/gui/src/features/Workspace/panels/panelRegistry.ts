import { lazy, type ComponentType } from "react";

import type { PanelKind } from "../surfaceKey";
import { FilesPanel, TerminalPanel } from "./PanelPlaceholder";

const GitPanel = lazy(() =>
  import("../GitPanel/GitPanel").then((module) => ({
    default: module.GitPanel,
  })),
);

export const PANEL_COMPONENTS: Record<PanelKind, ComponentType> = {
  files: FilesPanel,
  git: GitPanel,
  terminal: TerminalPanel,
};
