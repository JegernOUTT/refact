import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, it } from "vitest";
import { render, screen, within } from "../../../../utils/test-utils";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import { setBuddySnapshot } from "../../buddySlice";
import { BuddyHome } from "../../BuddyHome";
import { resetBuddyPersistentState } from "../../ui";
import {
  CONFIG_STATE,
  makeRuntimeEvent,
  makeHomeSnapshot as makeSnapshot,
  installBuddyHomeHandlers,
} from "../../__tests__/buddyHomeFixtures";

afterEach(() => resetBuddyPersistentState());

describe("BuddyHome", () => {
  it("renders enabled recent errors as a deduplicated, expandable list", async () => {
    installBuddyHomeHandlers();
    server.use(
      http.get("*/v1/buddy/operator/status", () =>
        HttpResponse.json({
          schema_version: 1,
          mode: "enabled",
          features: {
            storage: true,
            actions: true,
            supervision: false,
            improvement: false,
            editorial: false,
          },
          project: { project_id: "project", canonical_root: "/workspace" },
          worker: {
            worker_id: "worker",
            engine_instance_id: "engine",
            epoch: 1,
          },
        }),
      ),
    );
    const currentTime = Date.now();
    const duplicateSignature = {
      title: "Repeated failure",
      description: "Provider issue detected.",
      failure_summary: "Repeated failure",
      source: "test-provider",
      signal_type: "provider_failed",
    } as const;
    const events = [
      makeRuntimeEvent({
        id: "same-title",
        title: "Repeated warning",
        failure_summary: "Repeated warning",
        created_at: new Date(currentTime).toISOString(),
      }),
      ...Array.from({ length: 3 }, (_, index) =>
        makeRuntimeEvent({
          ...duplicateSignature,
          id: `duplicate-${index}`,
          created_at: new Date(currentTime - 1000).toISOString(),
        }),
      ),
      ...Array.from({ length: 25 }, (_, index) =>
        makeRuntimeEvent({
          id: `distinct-${index}`,
          title: `Distinct warning ${index}`,
          description: `Provider detail ${index}.`,
          failure_summary: `Additional detail ${index}`,
          source: `test-provider-${index}`,
          created_at: new Date(currentTime - 2000 - index * 1000).toISOString(),
        }),
      ),
    ];
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot({ runtime_queue: events })));

    const { user } = render(<BuddyHome />, { store });
    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("tab", { name: "Activity" }));

    const list = await screen.findByRole("list", { name: "Recent warnings" });
    expect(within(list).getByText("Repeated warning")).toBeInTheDocument();
    expect(
      within(list).getAllByText("Repeated warning", { exact: true }),
    ).toHaveLength(1);
    const repeated = within(list)
      .getByText("Repeated failure", { exact: true })
      .closest("li");
    if (!repeated) throw new Error("Expected a repeated warning row");
    expect(within(repeated).getByText("×3")).toBeInTheDocument();
    expect(within(repeated).getByText("now")).toBeInTheDocument();
    expect(within(list).getAllByText("now").length).toBeGreaterThan(0);
    expect(
      within(list).queryByText("Repeated warning: Repeated warning"),
    ).toBeNull();
    expect(within(list).getAllByRole("listitem")).toHaveLength(20);
    expect(
      screen.getByRole("button", { name: "Show more" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show more" }));
    expect(within(list).getAllByRole("listitem")).toHaveLength(27);
    const lastRow = within(list).getByText("Distinct warning 24").closest("li");
    if (!lastRow) throw new Error("Expected the last warning row");
    const details = within(lastRow).getByRole("button", {
      name: "Technical details",
    });
    expect(details).toHaveAttribute("aria-expanded", "false");
    await user.click(details);
    expect(details).toHaveAttribute("aria-expanded", "true");
    expect(
      within(lastRow).getByText(/Additional detail 24/),
    ).toBeInTheDocument();
  });
});
