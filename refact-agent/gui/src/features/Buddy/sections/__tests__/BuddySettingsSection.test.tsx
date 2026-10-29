import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../../utils/test-utils";
import { http, HttpResponse } from "msw";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { server } from "../../../../utils/mockServer";
import { setUpStore } from "../../../../app/store";
import { setBuddySnapshot } from "../../buddySlice";
import { resetBuddyPersistentState } from "../../ui";
import { BuddySettingsPanel } from "../../BuddySettingsPanel";
import { BuddyHome } from "../../BuddyHome";
import type { BuddyPulse, BuddySnapshot } from "../../types";

const operatorStatus = (
  mode: "disabled" | "enabled" | "unavailable",
  unavailableReason?: {
    kind: string;
    detail: string;
    retry_in_seconds?: number;
  },
) => ({
  schema_version: 1,
  mode,
  features: {
    storage: mode !== "disabled",
    actions: false,
    supervision: false,
    improvement: false,
    editorial: false,
  },
  project:
    mode === "enabled"
      ? { project_id: "fixture", canonical_root: "/fixture" }
      : null,
  worker:
    mode === "enabled"
      ? { worker_id: "worker", engine_instance_id: "engine", epoch: 1 }
      : null,
  ...(mode === "unavailable" && unavailableReason
    ? { unavailable_reason: unavailableReason }
    : {}),
});

// Existing legacy tests explicitly exercise the disabled compatibility contract.
beforeEach(() => {
  server.use(
    http.get("*/v1/buddy/operator/status", () =>
      HttpResponse.json(operatorStatus("disabled")),
    ),
  );
});

vi.mock("../../BuddyCharacter", async () => {
  const ReactModule = await vi.importActual<typeof import("react")>("react");

  return {
    BuddyCharacter: ({
      bubblePosition,
      randomizeBubblePosition,
      compactBubble,
      scenePose = "idle",
      sceneXPercent,
      sceneYPercent,
      sceneDepthScale,
      speechText,
      speechControls,
      onSpeechControl,
    }: {
      bubblePosition?: string;
      randomizeBubblePosition?: boolean;
      compactBubble?: boolean;
      scenePose?: string;
      sceneXPercent?: number;
      sceneYPercent?: number;
      sceneDepthScale?: number;
      speechText?: string | null;
      speechControls?: {
        id: string;
        label: string;
        action: string;
        action_param?: string;
        style: string;
      }[];
      onSpeechControl?: (control: {
        id: string;
        label: string;
        action: string;
        action_param?: string;
        style: string;
      }) => void;
    }) =>
      ReactModule.createElement(
        "div",
        {
          "data-bubble-position": bubblePosition,
          "data-depth-scale": sceneDepthScale,
          "data-compact-bubble": String(compactBubble),
          "data-pose": scenePose,
          "data-randomize-bubble-position": String(randomizeBubblePosition),
          "data-testid": "buddy-world-character",
          style:
            typeof sceneXPercent === "number" ||
            typeof sceneYPercent === "number"
              ? {
                  ...(typeof sceneXPercent === "number"
                    ? { left: `${sceneXPercent}%` }
                    : undefined),
                  ...(typeof sceneYPercent === "number"
                    ? { bottom: `${100 - sceneYPercent}%` }
                    : undefined),
                }
              : undefined,
        },
        speechText,
        ...(speechControls?.map((control) =>
          ReactModule.createElement(
            "button",
            {
              key: control.id,
              type: "button",
              onClick: () => onSpeechControl?.(control),
            },
            control.label,
          ),
        ) ?? []),
      ),
  };
});

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function makePulse(overrides?: Partial<BuddyPulse>): BuddyPulse {
  return {
    generated_at: "2024-01-01T00:00:00Z",
    tasks: { total: 3, stuck: 1, abandoned: 2, by_status: {} },
    trajectories: { total: 10, untitled: 2, oldest_age_days: 14 },
    memory: { total: 50, orphan: 5, stale_conflicts: 1 },
    providers: { defaults_ok: true, broken_refs: 0, quota_warnings: 0 },
    mcp: { total: 4, failing: 1, auth_expiring: 0 },
    customization: { modes: 3, skills: 2, commands: 1, subagents: 0, hooks: 0 },
    diagnostics: { last_hour: 7, top_error_types: ["model_not_found"] },
    git: { uncommitted_files: 5, diff_lines_4h: 120, branches: 3 },
    worktrees: {
      total_registered: 3,
      total_discovered: 1,
      total: 4,
      clean: 2,
      dirty: 1,
      unknown: 0,
      stale: 1,
      conflicted: 0,
      shared: 1,
      abandoned_clean: 2,
      changed_files: 3,
      additions: 10,
      deletions: 2,
      missing_registry_paths: 1,
      unregistered_cache_dirs: 1,
      merged_branches: 2,
    },
    ...overrides,
  };
}

function makeSnapshot(
  pulse?: BuddyPulse,
  overrides?: Partial<BuddySnapshot>,
): BuddySnapshot {
  return {
    state: {
      identity: { name: "Buddy", created_at: "", palette_index: 0 },
      progression: {
        stage: 0,
        stage_name: "Egg",
        level: 1,
        xp: 0,
        xp_next: 20,
      },
      skills: { unlocked: [], locked: [] },
      workflow_summaries: [],
      semantic: {
        mood: "idle",
        focus: "helping",
        headline: "",
        last_active: "",
      },
      recent_activities: [],
      suggestion_state: [],
      pet: {
        needs: {
          hunger: 80,
          energy: 85,
          hygiene: 80,
          boredom: 15,
          affection: 75,
        },
        condition: {
          sleeping: false,
          hungry: false,
          sleepy: false,
          dirty: false,
          bored: false,
          lonely: false,
        },
        evolution: {
          care_score: 0,
          neglect_score: 0,
          open_seconds: 0,
          last_evolved_at: null,
        },
      },
      personality: {
        archetype_id: "helper_sprite",
        archetype_label: "Helper Sprite",
        vibe: "Playful",
        summary: "An energetic helper.",
        prompt: "Playful",
        traits: {
          playfulness: 70,
          chaos: 35,
          sociability: 72,
          curiosity: 78,
          resilience: 66,
        },
      },
      active_quest: null,
      opportunities: [],
    },
    settings: {
      enabled: true,
      auto_diagnostics: true,
      auto_issue_creation: false,
      personality_prompt: null,
      autonomous_chats_enabled: true,
      proactive_enabled: true,
      message_observation_enabled: false,
      chat_reactions_enabled: false,
      housekeeping_enabled: true,
      humor_enabled: true,
      humor_level: "light",
      autonomy_level: "suggest",
      quiet_mode: false,
      daily_digest_hour: 18,
      observers: {
        task_health: true,
        trajectory_clutter: true,
        chat_pattern: false,
        customization_drift: true,
        memory_garden: true,
        mcp_auth: true,
        git_pressure: true,
        diagnostic_cluster: true,
        provider_health: true,
      },
    },
    enabled: true,
    pulse,
    ...overrides,
  };
}

beforeEach(() => {
  resetBuddyPersistentState();
});

describe("BuddyHome_renders_all_sections", () => {
  it("settings section uses a CSS class wrapper when settings toggled on", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("tab", { name: "Settings" }));

    const settingsSection = await screen.findByTestId(
      "buddy-home-settings-section",
    );
    expect(settingsSection).toBeInTheDocument();
    expect(settingsSection).not.toHaveAttribute("style");
  });

  it("reaches the undo surface for applied changes through the settings tab", async () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({
          opportunities: [
            {
              id: "opp-dup-guard",
              kind: "memory_ops_batch",
              priority: "normal",
              summary: "Merge 12 near-duplicate memories",
              detail: "",
              confidence: 0.9,
              actions: [],
              created_at: "2026-07-03T06:00:00Z",
            },
          ],
        }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
      http.get("*/v1/buddy/receipts", () =>
        HttpResponse.json({
          receipts: [
            {
              id: "receipt-1",
              action_kind: "apply_config_patch",
              target_path: ".refact/knowledge/handbook/testing.md",
              pre_image: null,
              created_at: "2026-07-03T06:00:00Z",
              undone: false,
            },
          ],
        }),
      ),
    );

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("tab", { name: "Settings" }));

    const settingsSection = await screen.findByTestId(
      "buddy-home-settings-section",
    );
    const appliedChanges = await screen.findByTestId(
      "buddy-applied-changes-card",
    );
    expect(settingsSection).toContainElement(appliedChanges);
    expect(
      screen.getByRole("button", {
        name: "Undo .refact/knowledge/handbook/testing.md",
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Merge 12 near-duplicate memories"),
    ).not.toBeInTheDocument();
  });
});

describe("BuddySettingsPanel_autosave", () => {
  afterEach(() => {
    vi.useRealTimers();
  });
  it("toggling a switch immediately sends partial patch to server", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json({
          ...makeSnapshot().settings,
          quiet_mode: true,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    await user.click(quietSwitch);

    expect(quietSwitch).toBeChecked();
    await waitFor(() => {
      expect(capturedBody).toEqual({ quiet_mode: true });
    });
  });

  it("rapid switch toggles keep the last value after out-of-order responses", async () => {
    const capturedBodies: unknown[] = [];
    const responseResolvers: ((settings: BuddySnapshot["settings"]) => void)[] =
      [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        const settings = await new Promise<BuddySnapshot["settings"]>(
          (resolve) => responseResolvers.push(resolve),
        );
        return HttpResponse.json(settings);
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    await user.click(quietSwitch);
    await waitFor(() => {
      expect(capturedBodies).toHaveLength(1);
    });
    expect(quietSwitch).toBeChecked();

    await user.click(quietSwitch);
    await waitFor(() => {
      expect(capturedBodies).toHaveLength(2);
    });
    expect(quietSwitch).not.toBeChecked();
    expect(capturedBodies).toEqual([
      { quiet_mode: true },
      { quiet_mode: false },
    ]);

    responseResolvers[1]?.({ ...makeSnapshot().settings, quiet_mode: false });
    await waitFor(() => {
      expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(false);
    });

    responseResolvers[0]?.({ ...makeSnapshot().settings, quiet_mode: true });
    await new Promise((resolve) => window.setTimeout(resolve, 0));

    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(false);
    expect(quietSwitch).not.toBeChecked();
  });

  it("clicking a segmented enum button immediately sends partial patch", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json({
          ...makeSnapshot().settings,
          humor_level: "normal",
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const normalButton = screen.getByRole("radio", { name: /normal/i });
    await user.click(normalButton);

    expect(normalButton).toBeChecked();
    await waitFor(() => {
      expect(capturedBody).toEqual({ humor_level: "normal" });
    });
  });

  it("flushes the Settings number draft when switching sections without blur", async () => {
    const captured: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        const patch = await request.json();
        captured.push(patch);
        return HttpResponse.json({
          ...makeSnapshot().settings,
          ...(patch as object),
        });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));
    render(<BuddyHome />, { store });
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Settings" }), {
      button: 0,
      ctrlKey: false,
    });
    const input = screen.getByRole("spinbutton", {
      name: /daily digest hour/i,
    });
    input.focus();
    fireEvent.change(input, { target: { value: "7" } });
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Home" }), {
      button: 0,
      ctrlKey: false,
    });
    await waitFor(() => expect(captured).toEqual([{ daily_digest_hour: 7 }]));
  });

  it("daily digest saves valid and empty partial patches", async () => {
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        const body = (await request.json()) as {
          daily_digest_hour?: number | null;
        };
        capturedBodies.push(body);
        return HttpResponse.json({
          ...makeSnapshot().settings,
          daily_digest_hour:
            "daily_digest_hour" in body
              ? body.daily_digest_hour
              : makeSnapshot().settings.daily_digest_hour,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const digestInput = screen.getByRole("spinbutton", {
      name: /daily digest hour/i,
    });
    fireEvent.change(digestInput, { target: { value: "7" } });
    expect(digestInput).toHaveValue(7);
    await new Promise((resolve) => window.setTimeout(resolve, 0));
    expect(capturedBodies).toEqual([]);

    fireEvent.blur(digestInput);
    await waitFor(() => {
      expect(capturedBodies).toEqual([{ daily_digest_hour: 7 }]);
    });

    fireEvent.change(digestInput, { target: { value: "" } });
    fireEvent.keyDown(digestInput, { key: "Enter" });
    fireEvent.blur(digestInput);

    expect(digestInput).toHaveValue(null);
    await waitFor(() => {
      expect(capturedBodies).toEqual([
        { daily_digest_hour: 7 },
        { daily_digest_hour: null },
      ]);
    });
  });

  it.each(["1e2", "7.5", "24", "-1"])(
    "daily digest invalid value %s does not save or clobber usable state",
    (invalidValue) => {
      const capturedBodies: unknown[] = [];
      server.use(
        http.post("*/v1/buddy/settings", async ({ request }) => {
          capturedBodies.push(await request.json());
          return HttpResponse.json(makeSnapshot().settings);
        }),
      );

      const store = setUpStore({ ...CONFIG_STATE });
      store.dispatch(setBuddySnapshot(makeSnapshot()));

      render(<BuddySettingsPanel />, { store });

      const digestInput = screen.getByRole("spinbutton", {
        name: /daily digest hour/i,
      });
      fireEvent.change(digestInput, { target: { value: invalidValue } });
      fireEvent.blur(digestInput);

      expect(capturedBodies).toEqual([]);
      expect(store.getState().buddy.snapshot?.settings.daily_digest_hour).toBe(
        18,
      );
      expect(digestInput).toHaveValue(18);
    },
  );

  it("mutation success updates Redux settings via onQueryStarted", async () => {
    const updatedSettings = {
      ...makeSnapshot().settings,
      humor_level: "normal" as const,
    };
    server.use(
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json(updatedSettings),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    await user.click(screen.getByRole("radio", { name: /normal/i }));

    await waitFor(() => {
      expect(store.getState().buddy.snapshot?.settings.humor_level).toBe(
        "normal",
      );
    });
  });

  it("prompt change after debounce period sends personality_prompt patch", async () => {
    vi.useFakeTimers();
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(makeSnapshot().settings);
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    try {
      render(<BuddySettingsPanel />, { store });

      const textarea = screen.getByRole("textbox", {
        name: /personality prompt/i,
      });
      fireEvent.change(textarea, { target: { value: "Be more chaotic" } });

      expect(capturedBody).toBeUndefined();

      await vi.advanceTimersByTimeAsync(750);

      await vi.waitFor(() => {
        expect(capturedBody).toEqual({
          personality_prompt: "Be more chaotic",
        });
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("prompt blur saves immediately without waiting for debounce", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json(makeSnapshot().settings);
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const textarea = screen.getByRole("textbox", {
      name: /personality prompt/i,
    });
    fireEvent.change(textarea, { target: { value: "Be calm" } });
    fireEvent.blur(textarea);

    await waitFor(() => {
      expect(capturedBody).toEqual({ personality_prompt: "Be calm" });
    });
  });

  it("delete all prompt text and debounce sends clear_personality_prompt flag", async () => {
    vi.useFakeTimers();
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          personality_prompt: null,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Custom personality text",
          },
        }),
      ),
    );

    try {
      render(<BuddySettingsPanel />, { store });

      const textarea = screen.getByRole("textbox", {
        name: /personality prompt/i,
      });
      fireEvent.change(textarea, { target: { value: "" } });

      expect(capturedBodies).toHaveLength(0);

      await vi.advanceTimersByTimeAsync(750);

      await vi.waitFor(() => {
        expect(capturedBodies).toEqual([{ clear_personality_prompt: true }]);
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it("delete all prompt text and blur sends clear_personality_prompt flag", async () => {
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          personality_prompt: null,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Custom personality text",
          },
        }),
      ),
    );

    render(<BuddySettingsPanel />, { store });

    const textarea = screen.getByRole("textbox", {
      name: /personality prompt/i,
    });
    fireEvent.change(textarea, { target: { value: "" } });
    fireEvent.blur(textarea);

    await waitFor(() => {
      expect(capturedBodies).toEqual([{ clear_personality_prompt: true }]);
    });
  });

  it("prompt draft survives unrelated live settings update before debounce fires", async () => {
    vi.useFakeTimers();
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          quiet_mode: true,
          personality_prompt: "Server prompt",
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Server prompt",
          },
        }),
      ),
    );

    try {
      render(<BuddySettingsPanel />, { store });

      const textarea = screen.getByRole("textbox", {
        name: /personality prompt/i,
      });
      fireEvent.focus(textarea);
      fireEvent.change(textarea, { target: { value: "Unsaved local prompt" } });
      store.dispatch(
        setBuddySnapshot(
          makeSnapshot(undefined, {
            settings: {
              ...makeSnapshot().settings,
              quiet_mode: true,
              personality_prompt: "Server prompt",
            },
          }),
        ),
      );

      expect(textarea).toHaveValue("Unsaved local prompt");

      await vi.advanceTimersByTimeAsync(750);

      await vi.waitFor(() => {
        expect(capturedBodies).toEqual([
          { personality_prompt: "Unsaved local prompt" },
        ]);
      });
      expect(textarea).toHaveValue("Unsaved local prompt");
    } finally {
      vi.useRealTimers();
    }
  });

  it("prompt clear button sends clear_personality_prompt flag", async () => {
    let capturedBody: unknown;
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBody = await request.json();
        return HttpResponse.json({
          ...makeSnapshot().settings,
          personality_prompt: null,
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Custom personality text",
          },
        }),
      ),
    );

    const { user } = render(<BuddySettingsPanel />, { store });

    const clearBtn = await screen.findByTestId("buddy-clear-prompt");
    await user.click(clearBtn);

    await waitFor(() => {
      expect(capturedBody).toEqual({ clear_personality_prompt: true });
    });
  });

  it("shows Saved status after successful mutation", async () => {
    server.use(
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json(makeSnapshot().settings),
      ),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    await user.click(screen.getByRole("switch", { name: /^housekeeping$/i }));

    await waitFor(() => {
      expect(screen.getByText("Saved")).toBeInTheDocument();
    });
  });

  it("renders active Buddy storage diagnostics when present", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          storage: {
            project_root: "/repo/root",
            buddy_dir: "/repo/root/.refact/buddy",
            settings_path: "/repo/root/.refact/buddy/settings.json",
          },
        }),
      ),
    );

    render(<BuddySettingsPanel />, { store });

    expect(screen.getByRole("button", { name: "Advanced" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    fireEvent.click(screen.getByRole("button", { name: "Advanced" }));
    expect(screen.getByText("Buddy's folder")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Copy settings file" }),
    ).toBeInTheDocument();
    expect(screen.getByText("/repo/root/.refact/buddy")).toBeInTheDocument();
    expect(
      screen.getByText("/repo/root/.refact/buddy/settings.json"),
    ).toBeInTheDocument();
  });

  it("tolerates missing Buddy storage diagnostics", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    fireEvent.click(screen.getByRole("button", { name: "Advanced" }));
    expect(screen.getByTestId("buddy-storage-diagnostics")).toHaveTextContent(
      "Storage details aren't available from this engine.",
    );
  });

  it("shows Failed status on save error without unhandled rejections", async () => {
    server.use(
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json({ error: "nope" }, { status: 500 }),
      ),
    );

    const unhandled = vi.fn();
    window.addEventListener("unhandledrejection", unhandled);

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    try {
      const { user } = render(<BuddySettingsPanel />, { store });

      await user.click(screen.getByRole("switch", { name: /^housekeeping$/i }));

      expect(await screen.findByRole("alert")).toHaveTextContent("Save failed");

      await new Promise((resolve) => window.setTimeout(resolve, 0));
      expect(unhandled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("unhandledrejection", unhandled);
    }
  });

  it("failed switch autosave rolls back the optimistic Redux and control state", async () => {
    const responseResolvers: ((response: Response) => void)[] = [];
    server.use(
      http.post(
        "*/v1/buddy/settings",
        () =>
          new Promise<Response>((resolve) => {
            responseResolvers.push(resolve);
          }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const housekeepingSwitch = screen.getByRole("switch", {
      name: /^housekeeping$/i,
    });
    await user.click(housekeepingSwitch);

    await waitFor(() => {
      expect(
        store.getState().buddy.snapshot?.settings.housekeeping_enabled,
      ).toBe(false);
    });
    expect(housekeepingSwitch).not.toBeChecked();

    responseResolvers[0]?.(
      HttpResponse.json({ error: "nope" }, { status: 500 }),
    );

    await waitFor(() => {
      expect(
        store.getState().buddy.snapshot?.settings.housekeeping_enabled,
      ).toBe(true);
    });
    expect(housekeepingSwitch).toBeChecked();
    expect(await screen.findByRole("alert")).toHaveTextContent("Save failed");
  });

  it("failed enabled autosave restores Buddy Home instead of trapping disabled UI", async () => {
    server.use(
      http.get("*/v1/buddy/opportunities", () =>
        HttpResponse.json({ opportunities: [] }),
      ),
      http.get("*/v1/buddy/conversations", () => HttpResponse.json([])),
      http.get("*/v1/stats/llm/summary", () =>
        HttpResponse.json({
          totals: { total_calls: 0, successful_calls: 0, total_tokens: 0 },
        }),
      ),
      http.get("*/v1/setup/status", () =>
        HttpResponse.json({ configured: true, reasons: [], detail: {} }),
      ),
      http.post("*/v1/buddy/settings", () =>
        HttpResponse.json({ error: "nope" }, { status: 500 }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot(makePulse())));

    const { user } = render(<BuddyHome />, { store });

    await screen.findByTestId("buddy-home-content");
    await user.click(screen.getByRole("tab", { name: "Settings" }));
    await user.click(screen.getByRole("switch", { name: /buddy enabled/i }));

    await waitFor(() => {
      expect(store.getState().buddy.snapshot?.settings.enabled).toBe(true);
    });
    expect(store.getState().buddy.snapshot?.enabled).toBe(true);
    expect(screen.getByTestId("buddy-home-content")).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-home-disabled")).not.toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /buddy enabled/i }),
    ).toBeChecked();
  });

  it("stale snapshot while a toggle is pending keeps the optimistic value visible", async () => {
    const responseResolvers: ((settings: BuddySnapshot["settings"]) => void)[] =
      [];
    server.use(
      http.post(
        "*/v1/buddy/settings",
        () =>
          new Promise<Response>((resolve) => {
            responseResolvers.push((settings) =>
              resolve(HttpResponse.json(settings)),
            );
          }),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    await user.click(quietSwitch);
    expect(quietSwitch).toBeChecked();

    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: { ...makeSnapshot().settings, quiet_mode: false },
        }),
      ),
    );

    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(true);
    expect(quietSwitch).toBeChecked();

    responseResolvers[0]?.({ ...makeSnapshot().settings, quiet_mode: true });
    await waitFor(() => {
      expect(screen.getByText("Saved")).toBeInTheDocument();
    });
  });

  it("out-of-order different setting requests keep pending settings visible", async () => {
    const capturedBodies: unknown[] = [];
    const responseResolvers: ((settings: BuddySnapshot["settings"]) => void)[] =
      [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        const settings = await new Promise<BuddySnapshot["settings"]>(
          (resolve) => responseResolvers.push(resolve),
        );
        return HttpResponse.json(settings);
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    const quietSwitch = await screen.findByRole("switch", {
      name: /quiet mode/i,
    });
    const housekeepingSwitch = screen.getByRole("switch", {
      name: /^housekeeping$/i,
    });
    await user.click(quietSwitch);
    await user.click(housekeepingSwitch);

    await waitFor(() => {
      expect(capturedBodies).toEqual([
        { quiet_mode: true },
        { housekeeping_enabled: false },
      ]);
    });
    expect(quietSwitch).toBeChecked();
    expect(housekeepingSwitch).not.toBeChecked();

    responseResolvers[0]?.({
      ...makeSnapshot().settings,
      quiet_mode: true,
      housekeeping_enabled: true,
    });
    await new Promise((resolve) => window.setTimeout(resolve, 0));

    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(true);
    expect(store.getState().buddy.snapshot?.settings.housekeeping_enabled).toBe(
      false,
    );
    expect(quietSwitch).toBeChecked();
    expect(housekeepingSwitch).not.toBeChecked();

    responseResolvers[1]?.({
      ...makeSnapshot().settings,
      quiet_mode: false,
      housekeeping_enabled: false,
    });
    await waitFor(() => {
      expect(screen.getByText("Saved")).toBeInTheDocument();
    });
    expect(store.getState().buddy.snapshot?.settings.quiet_mode).toBe(true);
    expect(store.getState().buddy.snapshot?.settings.housekeeping_enabled).toBe(
      false,
    );
  });

  it("slow failed request followed by fast success leaves status saved", async () => {
    const responseResolvers: ((response: Response) => void)[] = [];
    server.use(
      http.post(
        "*/v1/buddy/settings",
        () =>
          new Promise<Response>((resolve) => responseResolvers.push(resolve)),
      ),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    const { user } = render(<BuddySettingsPanel />, { store });

    await user.click(screen.getByRole("switch", { name: /quiet mode/i }));
    await user.click(screen.getByRole("switch", { name: /^housekeeping$/i }));
    await waitFor(() => {
      expect(responseResolvers).toHaveLength(2);
    });

    responseResolvers[1]?.(
      HttpResponse.json({
        ...makeSnapshot().settings,
        quiet_mode: false,
        housekeeping_enabled: false,
      }),
    );
    await waitFor(() => {
      expect(screen.getByText("Saved")).toBeInTheDocument();
    });

    responseResolvers[0]?.(
      HttpResponse.json({ error: "nope" }, { status: 500 }),
    );
    await new Promise((resolve) => window.setTimeout(resolve, 0));

    expect(screen.getByText("Saved")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("prompt focus and blur without edits does not save", async () => {
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json(makeSnapshot().settings);
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            personality_prompt: "Already saved prompt",
          },
        }),
      ),
    );

    render(<BuddySettingsPanel />, { store });

    const textarea = screen.getByRole("textbox", {
      name: /personality prompt/i,
    });
    fireEvent.focus(textarea);
    fireEvent.blur(textarea);

    await new Promise((resolve) => window.setTimeout(resolve, 0));
    expect(capturedBodies).toEqual([]);
    expect(screen.queryByText("Saving…")).not.toBeInTheDocument();
  });
});

describe("BuddySettingsPanel_renders_all_settings", () => {
  it("renders all required setting labels and controls", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    expect(
      screen.getByRole("switch", { name: /buddy enabled/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /quiet mode/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /auto diagnostics/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /auto issue creation/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /proactive suggestions/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /^chat pattern observation$/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /live chat reactions/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /^background chats$/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /^housekeeping$/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /^humor$/i }),
    ).toBeInTheDocument();

    expect(
      screen.getAllByRole("radio", { name: /off/i }).length,
    ).toBeGreaterThanOrEqual(1);
    expect(screen.getByRole("radio", { name: /light/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /normal/i })).toBeInTheDocument();

    expect(screen.getByRole("radio", { name: /observe/i })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /suggest/i })).toBeInTheDocument();
    expect(
      screen.getByRole("radio", { name: /safe auto/i }),
    ).toBeInTheDocument();

    expect(
      screen.getByRole("textbox", { name: /personality prompt/i }),
    ).toBeInTheDocument();

    expect(
      screen.getByRole("spinbutton", { name: /daily digest hour/i }),
    ).toBeInTheDocument();
  });

  it("renders all observer toggle labels", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    expect(
      screen.getByRole("switch", { name: /task health/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /^chat clutter$/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: "Chat patterns" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /customization drift/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /memory garden/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /mcp sign-in/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /uncommitted changes/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: "Repeated errors" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: /provider health/i }),
    ).toBeInTheDocument();
  });

  it("shows speech intents as allowed switches and writes the same muted_intents", async () => {
    const intentLabels = [
      ["humor", "Humor"],
      ["suggestion", "Suggestions"],
      ["insight", "Insights"],
      ["win", "Wins"],
      ["error_alert", "Error alerts"],
      ["greeting", "Greetings"],
      ["tour", "Tours"],
      ["milestone", "Milestones"],
      ["memory_pulse_commentary", "Memory pulse commentary"],
      ["quest_accept", "Quest acceptance"],
      ["quest_complete", "Quest completion"],
      ["chat_reaction", "Chat reactions"],
    ] as const;
    const opaqueRule = "rule-0123456789abcdef0123456789abcdef";
    const opaqueWorkflow = "12345678-1234-1234-1234-123456789abc";
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          muted_intents: intentLabels
            .map(([intent]) => intent)
            .filter((intent) => intent !== "error_alert"),
        });
      }),
    );

    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: {
            ...makeSnapshot().settings,
            muted_intents: intentLabels.map(([intent]) => intent),
          },
          state: {
            ...makeSnapshot().state,
            muted_rules: [opaqueRule, "buddy_memory_garden"],
            workflow_summaries: [
              {
                workflow_id: opaqueWorkflow,
                last_run: null,
                run_count: 1,
                last_outcome: null,
              },
            ],
          },
        }),
      ),
    );

    const { user } = render(<BuddySettingsPanel />, { store });
    const panel = screen.getByTestId("buddy-settings-panel");
    const mutedIntents = within(screen.getByTestId("buddy-muted-intents"));

    for (const [intent, label] of intentLabels) {
      expect(
        within(panel).getByRole("switch", {
          name: `Allow ${label.toLowerCase()}`,
        }),
      ).not.toBeChecked();
      expect(mutedIntents.getByText(label)).toBeInTheDocument();
      expect(label).not.toMatch(/[a-z]+_[a-z]+/);
      if (intent.includes("_"))
        expect(
          within(panel).queryByRole("switch", { name: `Allow ${intent}` }),
        ).toBeNull();
    }
    const switchNames = within(panel)
      .getAllByRole("switch")
      .map((toggle) => toggle.getAttribute("aria-label") ?? "");
    for (const name of switchNames) {
      expect(name).not.toMatch(/[a-z]+_[a-z]+/);
      expect(name).not.toMatch(/[0-9a-f]{24,}/i);
      expect(name).not.toMatch(/[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}/i);
    }
    expect(within(panel).getByText("Saved rule")).toBeInTheDocument();
    expect(
      within(panel).getByRole("button", { name: "Unmute Saved rule" }),
    ).toBeInTheDocument();
    expect(
      within(screen.getByTestId("buddy-muted-rules")).getByText(
        "Memory garden",
      ),
    ).toBeInTheDocument();
    fireEvent.click(
      within(panel).getByRole("button", { name: /job telemetry/i }),
    );
    expect(
      within(screen.getByTestId("buddy-job-telemetry")).getByText(
        "No job runs recorded yet.",
      ),
    ).toBeInTheDocument();
    expect(within(panel).queryByText(opaqueRule)).toBeNull();
    expect(within(panel).queryByText(opaqueWorkflow)).toBeNull();
    expect(
      within(panel).queryByRole("button", { name: new RegExp(opaqueRule) }),
    ).toBeNull();
    expect(
      within(panel).queryByRole("switch", {
        name: /[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}/i,
      }),
    ).toBeNull();

    await user.click(
      within(panel).getByRole("switch", { name: "Allow error alerts" }),
    );
    await waitFor(() => expect(capturedBodies).toHaveLength(1));
    expect(capturedBodies).toEqual([
      {
        muted_intents: intentLabels
          .map(([intent]) => intent)
          .filter((intent) => intent !== "error_alert"),
      },
    ]);
  });
});

describe("BuddySettingsPanel_segmented_controls_accessible_state", () => {
  it("active humor_level radio is checked and others are unchecked", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const lightBtn = screen.getByRole("radio", { name: /light/i });
    const humorGroup = lightBtn.closest('[role="radiogroup"]');
    const offBtn = within(humorGroup as HTMLElement).getByRole("radio", {
      name: /off/i,
    });
    const normalBtn = screen.getByRole("radio", { name: /normal/i });

    expect(lightBtn).toBeChecked();
    expect(offBtn).not.toBeChecked();
    expect(normalBtn).not.toBeChecked();
  });

  it("active autonomy_level radio is checked and others are unchecked", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const suggestBtn = screen.getByRole("radio", { name: /suggest/i });
    const readOnlyBtn = screen.getByRole("radio", { name: /observe/i });
    const safeAutoBtn = screen.getByRole("radio", { name: /safe auto/i });

    expect(suggestBtn).toBeChecked();
    expect(readOnlyBtn).not.toBeChecked();
    expect(safeAutoBtn).not.toBeChecked();
  });
});

describe("BuddySettingsPanel_allowed_intents", () => {
  it("turning a topic off writes the same muted_intents value as the old mute switch", async () => {
    const capturedBodies: unknown[] = [];
    server.use(
      http.post("*/v1/buddy/settings", async ({ request }) => {
        capturedBodies.push(await request.json());
        return HttpResponse.json({
          ...makeSnapshot().settings,
          muted_intents: ["humor"],
        });
      }),
    );
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(
      setBuddySnapshot(
        makeSnapshot(undefined, {
          settings: { ...makeSnapshot().settings, muted_intents: [] },
        }),
      ),
    );

    const { user } = render(<BuddySettingsPanel />, { store });
    const humor = screen.getByRole("switch", { name: "Allow humor" });
    expect(humor).toBeChecked();

    await user.click(humor);

    await waitFor(() =>
      expect(capturedBodies).toEqual([{ muted_intents: ["humor"] }]),
    );
    expect(humor).not.toBeChecked();
  });

  it("has exactly one switch per setting and no ALL-CAPS group labels", () => {
    const store = setUpStore({ ...CONFIG_STATE });
    store.dispatch(setBuddySnapshot(makeSnapshot()));

    render(<BuddySettingsPanel />, { store });

    const panel = screen.getByTestId("buddy-settings-panel");
    const names = within(panel)
      .getAllByRole("switch")
      .map((toggle) => toggle.getAttribute("aria-label") ?? "");
    expect(new Set(names).size).toBe(names.length);
    for (const heading of within(panel).getAllByRole("heading")) {
      expect(heading.textContent).not.toMatch(/^[A-Z &/?]{4,}$/);
    }
    expect(
      within(panel).getByRole("heading", { name: "What Buddy may talk about" }),
    ).toBeInTheDocument();
  });
});
