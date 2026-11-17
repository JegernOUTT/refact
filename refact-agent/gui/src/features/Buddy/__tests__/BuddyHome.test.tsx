import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { setUpStore } from "../../../app/store";
import {
  beginBuddySettingsRequest,
  markBuddyNotificationSeen,
  resetBuddyForWorkspaceChange,
  setBuddySnapshot,
} from "../buddySlice";
import { push } from "../../Pages/pagesSlice";
import { BuddyHome } from "../BuddyHome";
import { setCurrentProjectInfo } from "../../Chat/currentProject";
import {
  readBuddyPersistentState,
  resetBuddyPersistentState,
  writeBuddyPersistentState,
} from "../ui";

import {
  CONFIG_STATE,
  makeSettings,
  makeHomeSnapshot as makeSnapshot,
  installBuddyHomeHandlers,
} from "./buddyHomeFixtures";

describe("BuddyHome", () => {
  it("resets view filter and disclosure state on project identity changes only", async () => {
    resetBuddyPersistentState();
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    const first = render(<BuddyHome />, { store });
    await screen.findByTestId("buddy-home-content");
    const keys = [
      "buddy.reports.view",
      "buddy.activity.filter",
      "buddy.home.background",
      "buddy.settings.details",
    ];
    act(() => keys.forEach((key) => writeBuddyPersistentState(key, "changed")));
    await first.user.click(screen.getByRole("tab", { name: "Play" }));
    await first.user.click(screen.getByRole("tab", { name: "Home" }));
    keys.forEach((key) =>
      expect(readBuddyPersistentState(key)).toBe("changed"),
    );
    act(() => {
      store.dispatch(
        setCurrentProjectInfo({ name: "Other", workspaceRoots: ["/other"] }),
      );
    });
    keys.forEach((key) =>
      expect(readBuddyPersistentState(key)).toBeUndefined(),
    );
    resetBuddyPersistentState();
  });

  it("renders every section of the legacy Buddy home from store and RTK Query data", async () => {
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddyHome />, { store });

    expect(await screen.findByTestId("buddy-home-content")).toBeInTheDocument();
    expect(
      screen.getByRole("tablist", { name: "Buddy sections" }),
    ).toBeInTheDocument();
    const overview = screen.getByTestId("buddy-home-overview");
    expect(within(overview).getByTestId("buddy-home-hero")).toBeInTheDocument();
    expect(within(overview).getByTestId("buddy-world")).toBeInTheDocument();
    expect(
      within(overview).getByRole("button", { name: "Talk to Buddy" }),
    ).toBeInTheDocument();
    expect(within(overview).getByText("Keeping watch")).toBeInTheDocument();
    expect(
      screen.queryByTestId("buddy-presence-avatar"),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("buddy-summary-strip")).toBeNull();
    expect(screen.queryByTestId("buddy-setup-strip")).toBeNull();
    expect(
      await within(overview).findByText("Recent Buddy chat"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Reports" }));
    expect(await screen.findByTestId("buddy-pulse-card")).toBeInTheDocument();
    expect(
      await screen.findByTestId("buddy-opportunities-feed"),
    ).toBeInTheDocument();
    expect(screen.getByText("Model config is broken")).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-world")).toBeNull();

    await user.click(screen.getByRole("tab", { name: "Activity" }));
    expect(
      await screen.findByTestId("buddy-activity-panel"),
    ).toBeInTheDocument();
    expect(screen.getByText("Task coach noticed pattern")).toBeInTheDocument();
    expect(
      await screen.findByTestId("buddy-recent-errors-panel"),
    ).toBeInTheDocument();
    expect(screen.getByText("Model unavailable")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Play" }));
    expect(screen.getByTestId("buddy-world")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-summary-strip")).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "Settings" }));
    expect(
      await screen.findByTestId("buddy-personality-panel"),
    ).toBeInTheDocument();
    expect(screen.getByText("Helper Sprite")).toBeInTheDocument();
    expect(screen.getByText("Project setup")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Run setup" }),
    ).toBeInTheDocument();
  });

  it("shows disabled state and enables Buddy through the settings mutation", async () => {
    let requestBody: unknown;
    const disabledSettings = makeSettings({ enabled: false });
    installBuddyHomeHandlers(disabledSettings);
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        requestBody = await request.json();
        return HttpResponse.json(makeSettings({ enabled: true }));
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot({ enabled: false, settings: disabledSettings }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    const disabledHome = await screen.findByTestId("buddy-home-disabled");
    expect(disabledHome).toHaveTextContent(
      "Buddy is disabled. Still here, just politely lurking.",
    );

    await user.click(
      within(disabledHome).getByRole("button", { name: "Enable Buddy" }),
    );

    await waitFor(() => {
      expect(requestBody).toEqual({ enabled: true });
    });
  });

  it("shows the loading state before the Buddy snapshot is loaded", () => {
    installBuddyHomeHandlers();

    render(<BuddyHome />, { preloadedState: CONFIG_STATE });

    expect(screen.getByText("Loading Buddy")).toBeInTheDocument();
  });

  it("resets workspace-scoped Buddy state while preserving seen notifications", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(markBuddyNotificationSeen("opp-1"));
    store.dispatch(
      beginBuddySettingsRequest({
        requestSeq: 1,
        keys: ["enabled"],
        patch: { enabled: false },
      }),
    );

    store.dispatch(resetBuddyForWorkspaceChange());

    const buddyState = store.getState().buddy;
    expect(buddyState.snapshot).toBeNull();
    expect(buddyState.loaded).toBe(false);
    expect(buddyState.pendingSettingsRequests).toEqual([]);
    expect(buddyState.seenNotificationIds["opp-1"]).toBeGreaterThan(0);
  });
});

describe("BuddyHome embedded route context", () => {
  it("retains its deep link after global navigation and embedded Back never pops pages", async () => {
    installBuddyHomeHandlers();
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    store.dispatch(push({ name: "buddy", draftId: "wrong-global-draft" }));
    const { user } = render(
      <BuddyHome
        embedded
        routeContext={{
          name: "buddy",
          workId: "pane-work",
          workProjectId: "pane-project",
        }}
      />,
      { store },
    );
    expect(await screen.findByTestId("buddy-home-content")).toHaveAttribute(
      "data-section",
      "reports",
    );
    act(() => {
      store.dispatch(push({ name: "chat" }));
    });
    const pages = store.getState().pages;
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(store.getState().pages).toEqual(pages);
    expect(screen.getByTestId("buddy-home-content")).toHaveAttribute(
      "data-section",
      "reports",
    );
    expect(screen.queryByText("wrong-global-draft")).not.toBeInTheDocument();
  });
});
