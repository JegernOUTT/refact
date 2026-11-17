import { describe, expect, it } from "vitest";
import { render, screen } from "../../../../utils/test-utils";

import { setUpStore } from "../../../../app/store";
import { setBuddySnapshot } from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";

import {
  CONFIG_STATE,
  makePulse,
  makeHomeSnapshot as makeSnapshot,
  installBuddyHomeHandlers,
} from "../../__tests__/buddyHomeFixtures";

describe("BuddyHome", () => {
  it("shows memory operation and candidate counters in the pulse card", async () => {
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot({
          pulse: makePulse({
            memory: {
              total: 50,
              orphan: 5,
              stale_conflicts: 1,
              merge_candidates: 2,
              archive_candidates: 3,
              review_candidates: 4,
              conflict_candidates: 1,
              pending_ops: 6,
              applied_ops: 7,
              failed_ops: 1,
            },
          }),
        }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("tab", { name: "Reports" }));

    expect(await screen.findByTestId("buddy-pulse-card")).toHaveTextContent(
      "50 docs · 5 orphan · 1 conflict · 6 pending · 7 applied · 1 failed · 10 candidates",
    );
  });
});
