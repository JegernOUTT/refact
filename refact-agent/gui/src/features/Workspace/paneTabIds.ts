export const paneTabId = (leafId: string, surfaceKey: string) =>
  `pane-tab-${encodeURIComponent(leafId)}-${encodeURIComponent(surfaceKey)}`;

export const panePanelId = (leafId: string, surfaceKey: string) =>
  `pane-panel-${encodeURIComponent(leafId)}-${encodeURIComponent(surfaceKey)}`;
