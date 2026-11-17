import { beforeEach, describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen } from "../../../../utils/test-utils";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import { BuddySettingsPanel } from "../../BuddySettingsPanel";
import { setBuddySnapshot } from "../../buddySlice";
import { resetBuddyPersistentState } from "../../ui";
import type { BuddySnapshot } from "../../types";
import { CONFIG_STATE } from "../../__tests__/buddyHomeFixtures";
import { makePolishSnapshot as makeSnapshot } from "../../__tests__/buddyHomeFixtures";

beforeEach(() => {
  resetBuddyPersistentState();
});

describe("buddy UI polish", () => {
  it("BuddySettingsPanel_renders_job_telemetry_table", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    const snapshot = makeSnapshot();
    snapshot.state.workflow_summaries = [
      {
        workflow_id: "refact_error_detective",
        last_run: "2026-07-01T10:00:00Z",
        run_count: 12,
        last_outcome: "success",
        llm_calls: 48,
        tokens_in: 125_000,
        tokens_out: 9_400,
        outputs: 11,
        last_output_at: "2026-07-01T10:05:00Z",
      },
      {
        workflow_id: "buddy_voice",
        last_run: null,
        run_count: 0,
        last_outcome: null,
        llm_calls: 3,
        tokens_in: 1_500,
        tokens_out: 120,
        outputs: 3,
        last_output_at: "2026-07-01T09:00:00Z",
      },
    ];
    store.dispatch(setBuddySnapshot(snapshot));

    const { user } = render(<BuddySettingsPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    const section = await screen.findByTestId("buddy-job-telemetry");
    const trigger = screen.getByRole("button", { name: /job telemetry/i });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    await user.click(trigger);
    expect(section).toHaveTextContent("Job telemetry");
    expect(section).toHaveTextContent("Error detective");
    expect(section).not.toHaveTextContent("refact_error_detective");
    expect(section).toHaveTextContent("134k tokens");
    expect(section).toHaveTextContent("Speech lines");
    expect(section).not.toHaveTextContent("buddy_voice");
    expect(
      section.querySelector("time[datetime='2026-07-01T10:05:00.000Z']"),
    ).not.toBeNull();
    expect(section).not.toHaveTextContent("2026-07-01 10:05");
  });

  it("BuddySettingsPanel_shows_empty_telemetry_placeholder", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    const section = await screen.findByTestId("buddy-job-telemetry");
    await user.click(screen.getByRole("button", { name: /job telemetry/i }));
    expect(section).toHaveTextContent("No job runs recorded yet");
  });

  it.each([
    ["missing", undefined, "Sent 0, skipped 0."],
    ["partial", { emitted: 4 }, "Sent 4, skipped 0."],
    ["zero", { emitted: 0, skipped: 0 }, "Sent 0, skipped 0."],
  ] as const)(
    "settings_never_renders_undefined when counts_by_result is %s",
    async (_, countsByResult, expectedStatus) => {
      const store = setUpStore({ ...CONFIG_STATE });
      const snapshot = makeSnapshot();
      snapshot.chat_reaction_debug = {
        recent_attempts: [],
        ...(countsByResult === undefined
          ? {}
          : { counts_by_result: countsByResult }),
      } as NonNullable<BuddySnapshot["chat_reaction_debug"]>;
      store.dispatch(setBuddySnapshot(snapshot));

      const { user } = render(<BuddySettingsPanel />, {
        preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
      });

      await user.click(screen.getByRole("button", { name: "Advanced" }));
      const diagnostics = await screen.findByTestId(
        "buddy-chat-reaction-diagnostics",
      );
      expect(diagnostics).not.toHaveTextContent("undefined");
      expect(diagnostics).toHaveTextContent(expectedStatus);
    },
  );

  it("settings_uses_the_live_buddy_name", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    const snapshot = makeSnapshot();
    snapshot.state.identity = { ...snapshot.state.identity, name: "Spark" };
    store.dispatch(setBuddySnapshot(snapshot));

    render(<BuddySettingsPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    expect(
      await screen.findByText(
        "Tune Spark's autonomy, observations, and tiny-chaos operating hours.",
      ),
    ).toBeVisible();
    expect(screen.getByText(/Spark reacts to your messages/)).toBeVisible();
    expect(screen.queryByText(/Pixel/)).not.toBeInTheDocument();
  });

  it("BuddySettingsPanel_shows_chat_observation_and_reactions_as_independent", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    const snapshot = makeSnapshot();
    snapshot.settings = {
      ...snapshot.settings,
      message_observation_enabled: false,
      chat_reactions_enabled: true,
      observers: {
        ...snapshot.settings.observers,
        chat_pattern: false,
      },
    };
    store.dispatch(setBuddySnapshot(snapshot));

    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        const body = (await request.json()) as Partial<
          BuddySnapshot["settings"]
        >;
        return HttpResponse.json({
          ...snapshot.settings,
          ...body,
        });
      }),
    );

    const { user } = render(<BuddySettingsPanel />, {
      preloadedState: { ...CONFIG_STATE, buddy: store.getState().buddy },
    });

    const patternToggle = await screen.findByTestId(
      "buddy-toggle-chat-pattern-observation",
    );
    const reactionsToggle = screen.getByTestId(
      "buddy-toggle-live-chat-reactions",
    );

    expect(
      screen.getByRole("switch", { name: /^chat pattern observation$/i }),
    ).toBe(patternToggle);
    expect(screen.getByRole("switch", { name: /^live chat reactions$/i })).toBe(
      reactionsToggle,
    );
    expect(screen.getByText("Chat pattern observation")).toBeInTheDocument();
    expect(screen.getByText("Live chat reactions")).toBeInTheDocument();
    expect(
      screen.getByText(/Independent from live chat reactions/i),
    ).toBeVisible();
    expect(screen.getByText(/Buddy reacts to your messages/i)).toBeVisible();
    expect(patternToggle).toHaveAttribute("data-state", "unchecked");
    expect(reactionsToggle).toHaveAttribute("data-state", "checked");

    await user.click(patternToggle);

    expect(patternToggle).toHaveAttribute("data-state", "checked");
    expect(reactionsToggle).toHaveAttribute("data-state", "checked");
  });
});
