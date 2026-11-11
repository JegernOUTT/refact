import { useSyncExternalStore } from "react";

import type { Actor, RemoteLink, Review } from "../../services/refact/review";

export function remoteLink(
  review: Review,
  kind: RemoteLink["kind"],
  localId: string,
): RemoteLink | null {
  return (
    review.remote_links?.find(
      (link) => link.kind === kind && link.local_id === localId,
    ) ?? null
  );
}

/** `github:alice` is shown as `alice`. */
export function externalAuthor(actor: Actor): string {
  const at = actor.id.indexOf(":");
  return at >= 0 ? actor.id.slice(at + 1) : actor.id;
}

const PUBLISH_KEY = "refact.review.publishComments";
const listeners = new Set<() => void>();

function readPublish(): boolean {
  try {
    return window.localStorage.getItem(PUBLISH_KEY) === "true";
  } catch {
    return false;
  }
}

export function setPublishComments(value: boolean) {
  try {
    if (value) window.localStorage.setItem(PUBLISH_KEY, "true");
    else window.localStorage.removeItem(PUBLISH_KEY);
  } catch {
    return;
  }
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Whether the user's own canvas comments also go to the pull request. */
export function usePublishComments(): boolean {
  return useSyncExternalStore(subscribe, readPublish, () => false);
}
