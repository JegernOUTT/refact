import { lazy, type ComponentType } from "react";

import type { CenterPanelKind } from "../surfaceKey";

const GitPanel = lazy(() =>
  import("../GitPanel/GitPanel").then((module) => ({
    default: module.GitPanel,
  })),
);

export const PANEL_COMPONENTS: Record<CenterPanelKind, ComponentType> = {
  git: GitPanel,
};
