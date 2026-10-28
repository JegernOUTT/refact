import React from "react";

const focusableSelector = [
  "a[href]",
  "area[href]",
  "button",
  "input",
  "select",
  "textarea",
  "summary",
  "iframe",
  "object",
  "embed",
  "audio[controls]",
  "video[controls]",
  "[contenteditable='true']",
  "[tabindex]",
].join(",");

const collapsedFocusOwners = new WeakMap<
  HTMLElement,
  { tabIndex: string | null; owners: Set<HTMLElement> }
>();

export function useCollapsedFocusGuard(
  ref: React.RefObject<HTMLElement>,
  active: boolean,
): void {
  React.useLayoutEffect(() => {
    const root = ref.current;
    if (!root || !active) return;

    const owned = new Set<HTMLElement>();
    const release = (element: HTMLElement) => {
      owned.delete(element);
      const state = collapsedFocusOwners.get(element);
      if (!state) return;
      state.owners.delete(root);
      if (state.owners.size > 0) return;
      if (state.tabIndex === null) element.removeAttribute("tabindex");
      else element.setAttribute("tabindex", state.tabIndex);
      collapsedFocusOwners.delete(element);
    };
    const sync = () => {
      for (const element of owned) {
        if (!root.contains(element)) release(element);
      }
      for (const element of root.querySelectorAll<HTMLElement>(
        focusableSelector,
      )) {
        if (!owned.has(element)) {
          const state = collapsedFocusOwners.get(element) ?? {
            tabIndex: element.getAttribute("tabindex"),
            owners: new Set<HTMLElement>(),
          };
          state.owners.add(root);
          collapsedFocusOwners.set(element, state);
          owned.add(element);
        }
        element.tabIndex = -1;
      }
    };

    root.setAttribute("inert", "");
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(root, { childList: true, subtree: true });
    return () => {
      observer.disconnect();
      root.removeAttribute("inert");
      for (const element of owned) release(element);
    };
  }, [active, ref]);
}
