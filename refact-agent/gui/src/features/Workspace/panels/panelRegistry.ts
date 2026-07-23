import { lazy, type ComponentType } from "react";

import type { CenterPanelKind } from "../surfaceKey";
import { FilesPanel } from "../FilesPanel";

const GitPanel = lazy(() =>
  import("../GitPanel/GitPanel").then((module) => ({
    default: module.GitPanel,
  })),
);

export const PANEL_COMPONENTS: Record<CenterPanelKind, ComponentType> = {
  files: FilesPanel,
  git: GitPanel,
};
