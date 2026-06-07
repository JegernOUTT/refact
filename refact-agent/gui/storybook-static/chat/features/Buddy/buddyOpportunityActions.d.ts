import type { BuddyAction, BuddyControl, BuddyOpportunity, CustomizationKind, MarketKind, PulseScope } from "./types";
export declare function actionLabel(action: BuddyAction): string;
export declare function opportunitySpeechText(opportunity: BuddyOpportunity): string;
export declare function opportunityActionControls(opportunity: BuddyOpportunity): BuddyControl[];
export declare function getOpportunityActionIndexFromControl(control: BuddyControl): number | null;
export declare function getOpportunityActionFromControl(control: BuddyControl, opportunity: BuddyOpportunity): BuddyAction | null;
export declare function getOpportunityDismissAction(opportunity: BuddyOpportunity): {
    action: BuddyAction;
    actionIndex: number;
};
export declare function humanizeCustomizationKind(kind: CustomizationKind): string;
export declare function humanizePulseScope(scope: PulseScope): string;
export declare function humanizeMarketKind(kind: MarketKind): string;
