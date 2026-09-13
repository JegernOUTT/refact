export type BuddyDeskLane = "needs" | "working" | "delivered" | "watching";
export type BuddyDeskState =
  | "proposed"
  | "verified"
  | "blocked"
  | "partial"
  | "watching";
export type SimulatedOutcome = "success" | "partial" | "failure";

export interface BuddyDeskAction {
  label: string;
  outcome: SimulatedOutcome;
  result: string;
}

export interface BuddyDeskItem {
  id: string;
  lane: BuddyDeskLane;
  state: BuddyDeskState;
  title: string;
  summary: string;
  evidence: string;
  artifact?: string;
  action?: BuddyDeskAction;
}

export const buddyDeskFixtures: readonly BuddyDeskItem[] = [
  {
    id: "provider-decision",
    lane: "needs",
    state: "blocked",
    title: "Choose a provider fallback",
    summary:
      "The fixture provider declined a model request; no setting was changed.",
    evidence:
      "Provider fixture: 429 quota exhausted · fallback policy needs approval.",
    action: {
      label: "Simulate fallback decision",
      outcome: "failure",
      result: "Simulated provider failure: no provider setting was changed.",
    },
  },
  {
    id: "responsive-check",
    lane: "working",
    state: "proposed",
    title: "Check the compact workspace",
    summary: "A proposed narrow-layout check is ready for review.",
    evidence:
      "Proposed scope: dashboard cards at 240px and 360px. No check has run.",
    action: {
      label: "Simulate layout check",
      outcome: "partial",
      result:
        "Simulated partial improvement: 360px passes; 240px still wraps one label.",
    },
  },
  {
    id: "verified-report",
    lane: "delivered",
    state: "verified",
    title: "Keyboard navigation report",
    summary:
      "Verified with tab order and visible focus in the fixture preview.",
    evidence: "Verified 2026-06-07 · 8 focusable controls reached in order.",
    artifact:
      "# Keyboard navigation report\n\n- Tab reaches each work item and its stable action.\n- Enter opens a readable detail without opening chat.\n- Escape is unnecessary: the detail remains in the same document.\n\nResult: verified in this simulated preview only.",
  },
  {
    id: "quiet-watch",
    lane: "watching",
    state: "watching",
    title: "Watch provider recovery",
    summary: "No change detected since the last fixture sample.",
    evidence:
      "No-change watch · sampled 10:04 · this is not a pending decision.",
  },
];

export const laneLabels: Record<BuddyDeskLane, string> = {
  needs: "Needs you",
  working: "Working on",
  delivered: "Delivered",
  watching: "Watching",
};
