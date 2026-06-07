import { type ChatMessages } from "../../services/refact/types";
import type { DiagnosticContext } from "./types";
export type BuddyInvestigationSource = "thread" | "runtime" | "diagnostic" | "suggestion" | "frontend";
export type BuddyInvestigationPromptInput = {
    triggerSource: BuddyInvestigationSource;
    triggerText: string;
    sourceChatId?: string;
    messages: ChatMessages;
    diagnostic?: DiagnosticContext | null;
    logs?: string | null;
    internalContext?: string | null;
    repoOwner?: string;
    repoName?: string;
};
export declare function isBuddyOverlaySuppressedIssue(text: string, diagnostic?: DiagnosticContext | null): boolean;
export declare function buildBuddyInvestigationTitle(triggerText: string): string;
export declare function buildBuddyInvestigationPrompt(input: BuddyInvestigationPromptInput): string;
