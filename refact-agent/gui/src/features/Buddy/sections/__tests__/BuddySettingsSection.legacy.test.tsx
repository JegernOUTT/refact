import { describe, expect, it } from "vitest";
import { render, screen } from "../../../../utils/test-utils";

import { setUpStore } from "../../../../app/store";
import { setBuddySnapshot } from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";

import {
  CONFIG_STATE,
  makeHomeSnapshot as makeSnapshot,
  installBuddyHomeHandlers,
} from "../../__tests__/buddyHomeFixtures";

describe("BuddyHome", () => {
  it("opens settings and personality from the trailing settings tab", async () => {
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddyHome />, { store });

    expect(await screen.findByTestId("buddy-home-content")).toBeInTheDocument();
    // Configuration trails the useful tabs, so neither panel is mounted until
    // the last tab is selected.
    expect(
      screen.queryByTestId("buddy-settings-panel"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("buddy-personality-panel"),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Settings" }));

    expect(
      await screen.findByTestId("buddy-settings-panel"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("buddy-personality-panel")).toBeInTheDocument();
  });
});
