import { useSyncExternalStore } from "react";

export type BuddyCompanionState = {
  open: boolean;
  attentionCount: number;
  unread: boolean;
};

const initialState: BuddyCompanionState = {
  open: false,
  attentionCount: 0,
  unread: false,
};

let state = initialState;
let railButton: HTMLButtonElement | null = null;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot() {
  return state;
}

export function updateBuddyCompanion(patch: Partial<BuddyCompanionState>) {
  const next = { ...state, ...patch };
  if (
    next.open === state.open &&
    next.attentionCount === state.attentionCount &&
    next.unread === state.unread
  )
    return;
  state = next;
  emit();
}

export function setBuddyOpen(open: boolean) {
  updateBuddyCompanion(open ? { open, unread: false } : { open });
}

export function toggleBuddyOpen() {
  setBuddyOpen(!state.open);
}

export function resetBuddyCompanion() {
  state = initialState;
  emit();
}

export function setBuddyRailButton(element: HTMLButtonElement | null) {
  railButton = element;
}

export function getBuddyRailButton() {
  return railButton;
}

export function focusBuddyRailButton() {
  railButton?.focus();
}

export function useBuddyCompanionState() {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
