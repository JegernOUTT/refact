export function getScrollParent(node: HTMLElement | null): HTMLElement | null {
  if (!node) return null;
  let el: HTMLElement | null = node.parentElement;
  while (el) {
    const { overflowY } = window.getComputedStyle(el);
    if (
      overflowY === "auto" ||
      overflowY === "scroll" ||
      overflowY === "overlay"
    ) {
      return el;
    }
    el = el.parentElement;
  }
  return null;
}
