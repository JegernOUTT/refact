import type { HTMLAttributes, SyntheticEvent } from "react";

const DIALOG_INTERACTIVE_SELECTOR = [
  "button",
  "input",
  "textarea",
  "select",
  "a[href]",
  "summary",
  '[contenteditable=""]',
  '[contenteditable="true"]',
  '[role="button"]',
  '[role="checkbox"]',
  '[role="combobox"]',
  '[role="link"]',
  '[role="listbox"]',
  '[role="menu"]',
  '[role="menuitem"]',
  '[role="option"]',
  '[role="radio"]',
  '[role="slider"]',
  '[role="spinbutton"]',
  '[role="switch"]',
  '[role="tab"]',
  '[role="textbox"]',
  "[data-radix-collection-item]",
].join(",");

type DialogCloseHandlers = Pick<
  HTMLAttributes<HTMLElement>,
  "onPointerDownCapture" | "onMouseDownCapture" | "onClickCapture"
>;

function targetElement(target: EventTarget | null): Element | null {
  if (target instanceof Element) return target;
  if (target instanceof Node) return target.parentElement;
  return null;
}

function isInteractiveDialogTarget(element: Element): boolean {
  if (element.closest(DIALOG_INTERACTIVE_SELECTOR)) return true;

  const label = element.closest("label");
  return Boolean(label?.querySelector(DIALOG_INTERACTIVE_SELECTOR));
}

export function closeDialogOnNonInteractiveEvent(
  event: SyntheticEvent<HTMLElement>,
  close: () => void,
): void {
  const element = targetElement(event.target);
  if (!element || isInteractiveDialogTarget(element)) return;

  event.preventDefault();
  event.stopPropagation();
  close();
}

export function dialogNonInteractiveCloseHandlers(
  close: () => void,
): DialogCloseHandlers {
  const handleClose = (event: SyntheticEvent<HTMLElement>) =>
    closeDialogOnNonInteractiveEvent(event, close);

  return {
    onPointerDownCapture: handleClose,
    onMouseDownCapture: handleClose,
    onClickCapture: handleClose,
  };
}
