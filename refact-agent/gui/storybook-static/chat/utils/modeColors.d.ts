export declare const MODE_BADGE_COLORS: readonly ["gray", "gold", "bronze", "brown", "yellow", "amber", "orange", "tomato", "red", "ruby", "crimson", "pink", "plum", "purple", "violet", "iris", "indigo", "blue", "cyan", "teal", "jade", "green", "grass", "lime", "mint", "sky"];
export type ModeBadgeColor = (typeof MODE_BADGE_COLORS)[number];
export declare function getModeColor(modeId: string | undefined): ModeBadgeColor;
