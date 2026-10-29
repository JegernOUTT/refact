import { writeBuddyPersistentState } from "../ui";

export const BUDDY_REPORTS_VIEW_KEY = "buddy.reports.view";
export const BUDDY_REPORTS_SELECTION_KEY = "buddy.reports.selected";

export type BuddyReportsSelection = {
  workId: string;
  projectId: string;
  endpointIdentity: string;
  view: BuddyReportsView;
};

export type BuddyReportsView =
  | "needs_you"
  | "working_on"
  | "delivered"
  | "watching";

export function requestBuddyReportsView(view: BuddyReportsView): void {
  writeBuddyPersistentState(BUDDY_REPORTS_VIEW_KEY, view);
}
