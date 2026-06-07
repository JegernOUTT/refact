import type { BuddyRuntimeEvent } from "./types";
export declare const HIGH_ERROR_BUBBLE_GRACE_MS = 30000;
export declare const CRITICAL_ERROR_BUBBLE_GRACE_MS = 75000;
export declare function isBuddyRuntimeEventVisible(event: BuddyRuntimeEvent | null | undefined, nowMs?: number): event is BuddyRuntimeEvent;
export declare function isErrorRuntimeEvent(event: BuddyRuntimeEvent): boolean;
export declare function isFreshErrorWithinGrace(event: BuddyRuntimeEvent, nowMs?: number): boolean;
