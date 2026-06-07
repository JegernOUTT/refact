import type { BuddyAction, BuddyOpportunity } from "../types";
export declare function formatOpportunityActionError(error: unknown): string;
export declare function useExecuteBuddyAction(): (action: BuddyAction, opp: BuddyOpportunity | null, actionIndex: number) => Promise<void>;
