import type React from "react";
import { createContext, useContext } from "react";

export type ScrollAnchorTrigger =
  | HTMLElement
  | React.SyntheticEvent
  | Event
  | null
  | undefined;

export interface PreserveScrollAnchorOptions {
  trigger?: ScrollAnchorTrigger;
  settleElement?: () => HTMLElement | null;
}

export type PreserveScrollAnchor = (
  mutate: () => void,
  options?: PreserveScrollAnchorOptions,
) => void;
export type PrepareScrollAnchor = (trigger?: ScrollAnchorTrigger) => void;

export interface ChatScrollAnchorValue {
  preserveScrollAnchor: PreserveScrollAnchor;
  prepareScrollAnchor: PrepareScrollAnchor;
}

export const ChatScrollAnchorContext = createContext<ChatScrollAnchorValue>({
  preserveScrollAnchor: (mutate) => mutate(),
  prepareScrollAnchor: () => undefined,
});

export function useChatScrollAnchor(): PreserveScrollAnchor {
  return useContext(ChatScrollAnchorContext).preserveScrollAnchor;
}

export function usePrepareChatScrollAnchor(): PrepareScrollAnchor {
  return useContext(ChatScrollAnchorContext).prepareScrollAnchor;
}

export interface ScrollAnchorSnapshot {
  element: HTMLElement;
  key: string | null;
  topOffset: number;
  fallback?: { key: string; topOffset: number } | null;
}

const ANCHOR_SELECTOR = "[data-chat-scroll-anchor-item='true']";
const BOTTOM_TOLERANCE_PX = 24;
export const RESTORE_FRAME_COUNT = 18;
export const RESTORE_MAX_MS = 900;

function isNearBottom(scroller: HTMLElement): boolean {
  return (
    scroller.scrollTop + scroller.clientHeight >=
    scroller.scrollHeight - BOTTOM_TOLERANCE_PX
  );
}

export function resolveAnchorTrigger(
  trigger: ScrollAnchorTrigger,
): HTMLElement | null {
  if (!trigger) return null;
  if (trigger instanceof HTMLElement) return trigger;
  const target =
    "currentTarget" in trigger && trigger.currentTarget instanceof HTMLElement
      ? trigger.target instanceof HTMLElement
        ? trigger.target
        : trigger.currentTarget
      : trigger.target;
  return target instanceof HTMLElement ? target : null;
}

function captureTriggerAnchor(
  scroller: HTMLElement,
  trigger: HTMLElement,
): ScrollAnchorSnapshot | null {
  if (!scroller.contains(trigger)) return null;
  const scrollerRect = scroller.getBoundingClientRect();
  const rect = trigger.getBoundingClientRect();
  const item = trigger.closest<HTMLElement>(ANCHOR_SELECTOR);
  const visibilityRect = item ? item.getBoundingClientRect() : rect;
  if (
    visibilityRect.bottom <= scrollerRect.top ||
    visibilityRect.top >= scrollerRect.bottom
  ) {
    return null;
  }
  const key = item?.dataset.chatScrollAnchorKey ?? null;
  return {
    element: trigger,
    key: null,
    topOffset: rect.top - scrollerRect.top,
    fallback:
      item && key
        ? {
            key,
            topOffset: item.getBoundingClientRect().top - scrollerRect.top,
          }
        : null,
  };
}

export function captureScrollAnchor(
  scroller: HTMLElement,
  trigger?: ScrollAnchorTrigger,
): ScrollAnchorSnapshot | null {
  const triggerElement = resolveAnchorTrigger(trigger);
  if (triggerElement) {
    const snapshot = captureTriggerAnchor(scroller, triggerElement);
    if (snapshot) return snapshot;
  }

  if (isNearBottom(scroller)) return null;

  const scrollerRect = scroller.getBoundingClientRect();
  const anchorItems = scroller.querySelectorAll<HTMLElement>(ANCHOR_SELECTOR);

  let bestAnchor: ScrollAnchorSnapshot | null = null;
  let bestVisibleHeight = 0;

  for (const element of anchorItems) {
    const rect = element.getBoundingClientRect();
    if (rect.bottom <= scrollerRect.top || rect.top >= scrollerRect.bottom) {
      continue;
    }

    const visibleTop = Math.max(rect.top, scrollerRect.top);
    const visibleBottom = Math.min(rect.bottom, scrollerRect.bottom);
    const visibleHeight = visibleBottom - visibleTop;
    if (visibleHeight <= bestVisibleHeight) continue;

    bestAnchor = {
      element,
      key: element.dataset.chatScrollAnchorKey ?? null,
      topOffset: rect.top - scrollerRect.top,
    };
    bestVisibleHeight = visibleHeight;
  }

  return bestAnchor;
}

function findItemByKey(scroller: HTMLElement, key: string): HTMLElement | null {
  const anchorItems = scroller.querySelectorAll<HTMLElement>(ANCHOR_SELECTOR);
  for (const element of anchorItems) {
    if (element.dataset.chatScrollAnchorKey === key) return element;
  }
  return null;
}

function findAnchorTarget(
  scroller: HTMLElement,
  snapshot: ScrollAnchorSnapshot,
): { element: HTMLElement; topOffset: number } | null {
  if (snapshot.element.isConnected && scroller.contains(snapshot.element)) {
    return { element: snapshot.element, topOffset: snapshot.topOffset };
  }
  if (snapshot.key) {
    const element = findItemByKey(scroller, snapshot.key);
    if (element) return { element, topOffset: snapshot.topOffset };
  }
  if (snapshot.fallback) {
    const element = findItemByKey(scroller, snapshot.fallback.key);
    if (element) return { element, topOffset: snapshot.fallback.topOffset };
  }
  return null;
}

export function restoreScrollAnchor(
  scroller: HTMLElement,
  snapshot: ScrollAnchorSnapshot,
): boolean {
  const target = findAnchorTarget(scroller, snapshot);
  if (!target) return false;

  const scrollerRect = scroller.getBoundingClientRect();
  const rect = target.element.getBoundingClientRect();
  const delta = rect.top - scrollerRect.top - target.topOffset;

  if (Math.abs(delta) < 0.5) return true;

  scroller.scrollTop += delta;
  return true;
}

export interface ScheduleScrollAnchorOptions {
  settleElement?: () => HTMLElement | null;
  maxMs?: number;
  onDone?: () => void;
}

export function scheduleScrollAnchorRestore(
  scroller: HTMLElement,
  snapshot: ScrollAnchorSnapshot,
  onRestoreFrame?: () => void,
  options: ScheduleScrollAnchorOptions = {},
): () => void {
  const { settleElement, maxMs = RESTORE_MAX_MS, onDone } = options;
  let frame = 0;
  let animationFrame = 0;
  let finished = false;
  let settled = !settleElement;

  const restore = () => {
    restoreScrollAnchor(scroller, snapshot);
    onRestoreFrame?.();
  };

  const finish = () => {
    if (finished) return;
    finished = true;
    window.cancelAnimationFrame(animationFrame);
    window.clearTimeout(timeout);
    scroller.removeEventListener("transitionend", handleTransitionEnd);
    onDone?.();
  };

  const handleTransitionEnd = (event: Event) => {
    const element = settleElement?.();
    if (!element || event.target !== element) return;
    restore();
    settled = true;
    if (frame >= RESTORE_FRAME_COUNT) finish();
  };

  const restoreFrame = () => {
    if (finished) return;
    restore();
    frame += 1;
    if (frame >= RESTORE_FRAME_COUNT && settled) {
      finish();
      return;
    }
    animationFrame = window.requestAnimationFrame(restoreFrame);
  };

  const timeout = window.setTimeout(() => {
    if (finished) return;
    restore();
    finish();
  }, maxMs);

  if (settleElement) {
    scroller.addEventListener("transitionend", handleTransitionEnd);
  }
  animationFrame = window.requestAnimationFrame(restoreFrame);

  return finish;
}
