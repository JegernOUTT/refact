import type { AppDispatch } from "../../app/store";
import type { BuddyControl, BuddyPage, DraftKind } from "./types";
import type { DiagnosticContext } from "./types";
/**
 * Central executor for all Buddy control actions.
 *
 * Every Buddy surface (BuddyHome, BuddyPanel, BuddySpeechCloud,
 * NavigationRequest handler) must route through this single function
 * so that action semantics are defined in exactly one place.
 */
export declare function executeBuddyAction(ctrl: BuddyControl, dispatch: AppDispatch, investigation?: {
    triggerText: string;
    triggerSource: "thread" | "runtime" | "diagnostic" | "suggestion" | "frontend";
    sourceChatId?: string;
    diagnostic?: DiagnosticContext | null;
}): Promise<void>;
export declare function navigateFromBuddyPage(page: BuddyPage, dispatch: AppDispatch): void;
export declare function routeDraftByKind(result: {
    draft_kind: DraftKind;
    draft_id: string;
}, dispatch: AppDispatch): void;
/**
 * Central executor for engine-driven NavigationRequest events.
 *
 * Maps BuddyPage variants to actual GUI page dispatches.
 * Every NavigationRequest from the sidebar SSE must route through here.
 */
export declare function executeBuddyNavigation(page: BuddyPage, dispatch: AppDispatch): void;
