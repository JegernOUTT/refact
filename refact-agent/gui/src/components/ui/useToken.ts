import { useSyncExternalStore } from "react";

const OBSERVED_ATTRIBUTES = ["class", "data-appearance", "data-host"];

let version = 0;
let observer: MutationObserver | null = null;
let mediaQueries: MediaQueryList[] = [];
let rafId: number | null = null;
const subscribers = new Set<() => void>();

function normalizeTokenName(name: string): string {
  return name.startsWith("--") ? name : `--${name}`;
}

function canUseDOM(): boolean {
  return typeof window !== "undefined" && typeof document !== "undefined";
}

function readToken(name: string): string {
  if (!canUseDOM()) return "";
  return window
    .getComputedStyle(document.documentElement)
    .getPropertyValue(normalizeTokenName(name))
    .trim();
}

function emitChange(): void {
  version += 1;
  subscribers.forEach((subscriber) => subscriber());
}

function scheduleChange(): void {
  if (!canUseDOM()) return;

  if (rafId !== null) {
    window.cancelAnimationFrame(rafId);
  }

  rafId = window.requestAnimationFrame(() => {
    rafId = null;
    emitChange();
  });
}

function addMediaListener(query: MediaQueryList): void {
  query.addEventListener("change", scheduleChange);
}

function removeMediaListener(query: MediaQueryList): void {
  query.removeEventListener("change", scheduleChange);
}

function startTokenWatch(): void {
  if (!canUseDOM() || observer) return;

  observer = new MutationObserver(scheduleChange);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: OBSERVED_ATTRIBUTES,
  });

  observer.observe(document.body, {
    attributes: true,
    attributeFilter: OBSERVED_ATTRIBUTES,
  });

  if (typeof window.matchMedia === "function") {
    mediaQueries = [
      window.matchMedia("(prefers-color-scheme: dark)"),
      window.matchMedia("(prefers-color-scheme: light)"),
    ];
    mediaQueries.forEach(addMediaListener);
  }
}

function stopTokenWatch(): void {
  if (rafId !== null && canUseDOM()) {
    window.cancelAnimationFrame(rafId);
    rafId = null;
  }

  observer?.disconnect();
  observer = null;
  mediaQueries.forEach(removeMediaListener);
  mediaQueries = [];
}

function subscribe(onStoreChange: () => void): () => void {
  subscribers.add(onStoreChange);
  startTokenWatch();

  return () => {
    subscribers.delete(onStoreChange);
    if (subscribers.size === 0) {
      stopTokenWatch();
    }
  };
}

function getSnapshot(): number {
  return version;
}

function getServerSnapshot(): number {
  return 0;
}

function useTokenSnapshot(): number {
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

export function useToken(name: string): string {
  useTokenSnapshot();
  return readToken(name);
}

export function useTokens(names: string[]): Record<string, string> {
  useTokenSnapshot();

  return names.reduce<Record<string, string>>((tokens, name) => {
    tokens[name] = readToken(name);
    return tokens;
  }, {});
}
