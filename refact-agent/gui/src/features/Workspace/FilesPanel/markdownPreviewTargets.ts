const INTERACTIVE_SELECTOR =
  'a, button, input, textarea, select, label, iframe, svg, canvas, [role="button"], [contenteditable="true"], pre, .mermaid, [data-mermaid], [data-artifact], [data-svg-block]';

export const isInteractiveTarget = (
  target: EventTarget | null,
  container: HTMLElement,
): boolean => {
  if (!(target instanceof Element)) return false;
  const interactiveElement = target.closest(INTERACTIVE_SELECTOR);
  return interactiveElement !== null && container.contains(interactiveElement);
};
