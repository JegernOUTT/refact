import { render, screen, within } from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import { setBuddySnapshot } from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";

import {
  CONFIG_STATE,
  operatorStatus,
  makePulse,
  makeSnapshot,
} from "../../__tests__/buddyHomeFixtures";

// Existing legacy tests explicitly exercise the disabled compatibility contract.
beforeEach(() => {
  server.use(
    http.get("*/v1/buddy/operator/status", () =>
      HttpResponse.json(operatorStatus("disabled")),
    ),
  );
});

vi.mock("../../BuddyCharacter", async () => {
  const { mockBuddyCharacterModule } = await import(
    "../../__tests__/buddyHomeFixtures"
  );
  return mockBuddyCharacterModule();
});

describe("BuddyHome_operator_surface", () => {
  const operatorProject = operatorStatus("enabled").project;
  const envelope = (data: unknown) => ({
    schema_version: 1,
    project: operatorProject,
    data,
    next_cursor: null,
  });
  const surfaceHandlers = [
    http.get("*/v1/buddy/opportunities", () =>
      HttpResponse.json({ opportunities: [] }),
    ),
    http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
    http.get("*/v1/stats/llm/summary", () =>
      HttpResponse.json({
        totals: { total_calls: 50, successful_calls: 48, total_tokens: 900 },
      }),
    ),
    http.get("*/v1/setup/status", () =>
      HttpResponse.json({ configured: true, reasons: [], detail: {} }),
    ),
  ];
  it("renders Play as a section with one header action and the scene first", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    const content = await screen.findByTestId("buddy-home-content");
    content.scrollTop = 900;
    await user.click(screen.getByRole("tab", { name: "Play" }));
    expect(content.scrollTop).toBe(0);
    const play = screen.getByTestId("buddy-home-play");
    const section = within(play).getByRole("region", { name: "Play" });
    expect(
      within(section).getByRole("heading", { level: 2, name: "Play" }),
    ).toBeInTheDocument();
    const header = section.querySelector<HTMLElement>("header");
    if (header === null) throw new Error("missing section header");
    expect(within(header).getAllByRole("button")).toHaveLength(1);
    expect(
      within(header).getByRole("button", {
        name: "Reset scene",
      }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Leave Play" })).toBeNull();
    const hero = within(section).getByTestId("buddy-home-hero");
    expect(within(hero).getByTestId("buddy-world")).toBeInTheDocument();
    expect(
      hero.compareDocumentPosition(screen.getByTestId("buddy-summary-strip")),
    ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(screen.queryByTestId("buddy-home-saved-work")).toBeNull();
    expect(document.activeElement).toBe(
      screen.getByRole("tab", { name: "Play" }),
    );
  });

  it("keeps the speech bubble and scene controls in Play but not in the Home portrait", async () => {
    server.use(
      ...surfaceHandlers,
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json(operatorStatus("enabled")),
      ),
      http.get("*/v1/buddy/work", () => HttpResponse.json(envelope([]))),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    const homeWorld = await screen.findByTestId("buddy-world");
    expect(homeWorld).toHaveAttribute("data-portrait", "true");
    expect(within(homeWorld).queryAllByRole("button")).toHaveLength(0);
    expect(within(homeWorld).queryByRole("status")).toBeNull();
    expect(within(homeWorld).queryByTestId("buddy-world-controls")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Play" }));
    const playWorld = screen.getByTestId("buddy-world");
    expect(playWorld).not.toHaveAttribute("data-portrait");
    const controls = within(playWorld).getByTestId("buddy-world-controls");
    expect(within(controls).getByRole("status")).toBeInTheDocument();
    const dock = within(controls).getByRole("group", {
      name: "Buddy scene care actions",
    });
    for (const label of [
      "Play fetch with Buddy",
      "Feed Buddy",
      "Hunt bugs with Buddy",
      "Clean Buddy",
      "Let Buddy rest",
    ]) {
      const button = within(dock).getByRole("button", { name: label });
      expect(button.textContent).toBe("");
      expect(button.querySelector("svg")).not.toBeNull();
    }
    for (const hotspot of within(playWorld).getAllByRole("button", {
      name: /^Open /,
    })) {
      expect(hotspot).not.toHaveAttribute("title");
    }
  });
});
