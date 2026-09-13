import { useSyncExternalStore } from "react";

const key = "refact.buddy.text-presence.v1";
const eventName = "refact-buddy-text-presence";
let localValue = false;

function read(): boolean {
  try {
    return window.localStorage.getItem(key) === "true";
  } catch {
    return localValue;
  }
}
function subscribe(notify: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === key || event.key === null) notify();
  };
  window.addEventListener("storage", onStorage);
  window.addEventListener(eventName, notify);
  return () => {
    window.removeEventListener("storage", onStorage);
    window.removeEventListener(eventName, notify);
  };
}
function setTextPresence(enabled: boolean): void {
  localValue = enabled;
  try {
    window.localStorage.setItem(key, String(enabled));
  } catch {
    localValue = enabled;
  }
  window.dispatchEvent(new Event(eventName));
}
export function useBuddyTextPresence(): readonly [
  boolean,
  (enabled: boolean) => void,
] {
  return [useSyncExternalStore(subscribe, read, () => false), setTextPresence];
}
