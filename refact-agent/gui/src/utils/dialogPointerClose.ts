import type { PointerEvent } from "react";

const DIALOG_INTERACTIVE_SELECTOR = [
  "button",
  "input",
  "textarea",
  "select",
  "a[href]",
  "label",
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

export function closeDialogOnNonInteractivePointerDown(
  event: PointerEvent<HTMLElement>,
  close: () => void,
): void {
  const target = event.target;
  if (!(target instanceof Element)) return;
  if (target.closest(DIALOG_INTERACTIVE_SELECTOR)) return;

  event.preventDefault();
  event.stopPropagation();
  close();
}
