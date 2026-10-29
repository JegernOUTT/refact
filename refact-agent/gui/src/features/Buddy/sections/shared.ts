export type BuddyHomeSection =
  | "home"
  | "reports"
  | "activity"
  | "play"
  | "settings";

export type BuddyOperatorMode =
  | "loading"
  | "error"
  | "enabled"
  | "disabled"
  | "unavailable";

export type BuddyOperatorFailure = Extract<
  BuddyOperatorMode,
  "error" | "unavailable"
>;
