import { createContext, useContext } from "react";

export type PreserveScrollAnchor = (mutate: () => void) => void;
export type PrepareScrollAnchor = () => void;

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
}

const ANCHOR_SELECTOR = "[data-chat-scroll-anchor-item='true']";
const BOTTOM_TOLERANCE_PX = 24;
const RESTORE_FRAME_COUNT = 18;

function isNearBottom(scroller: HTMLElement): boolean {
  return (
    scroller.scrollTop + scroller.clientHeight >=
    scroller.scrollHeight - BOTTOM_TOLERANCE_PX
  );
}

export function captureScrollAnchor(
  scroller: HTMLElement,
): ScrollAnchorSnapshot | null {
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

function findAnchorElement(
  scroller: HTMLElement,
  snapshot: ScrollAnchorSnapshot,
): HTMLElement | null {
  if (snapshot.element.isConnected) return snapshot.element;
  if (!snapshot.key) return null;

  const anchorItems = scroller.querySelectorAll<HTMLElement>(ANCHOR_SELECTOR);
  for (const element of anchorItems) {
    if (element.dataset.chatScrollAnchorKey === snapshot.key) {
      return element;
    }
  }

  return null;
}

export function restoreScrollAnchor(
  scroller: HTMLElement,
  snapshot: ScrollAnchorSnapshot,
): boolean {
  const element = findAnchorElement(scroller, snapshot);
  if (!element) return false;

  const scrollerRect = scroller.getBoundingClientRect();
  const rect = element.getBoundingClientRect();
  const delta = rect.top - scrollerRect.top - snapshot.topOffset;

  if (Math.abs(delta) < 0.5) return true;

  scroller.scrollTop += delta;
  return true;
}

export function scheduleScrollAnchorRestore(
  scroller: HTMLElement,
  snapshot: ScrollAnchorSnapshot,
  onRestoreFrame?: () => void,
): () => void {
  let frame = 0;
  let animationFrame = 0;
  let cancelled = false;

  const restoreFrame = () => {
    if (cancelled) return;
    restoreScrollAnchor(scroller, snapshot);
    onRestoreFrame?.();
    frame += 1;
    if (frame < RESTORE_FRAME_COUNT) {
      animationFrame = window.requestAnimationFrame(restoreFrame);
    }
  };

  animationFrame = window.requestAnimationFrame(restoreFrame);

  return () => {
    cancelled = true;
    window.cancelAnimationFrame(animationFrame);
  };
}
