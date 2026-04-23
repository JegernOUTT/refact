export function isIdeHost(): boolean {
  return !!(window.acquireVsCodeApi ?? window.postIntellijMessage);
}
