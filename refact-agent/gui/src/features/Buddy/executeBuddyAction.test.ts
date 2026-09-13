import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { server } from "../../utils/mockServer";
import { setUpStore } from "../../app/store";
import { parseOperatorStatus } from "../../services/refact/buddyOperator";
import type { BuddyControl } from "./types";
import {
  buddyActionResolvesSuggestion,
  executeBuddyAction,
} from "./executeBuddyAction";

const DIRECT_PRODUCTION_CONTROLS = [
  {
    producer: "engine/src/buddy/jobs/greeting.rs + buddy_briefing.rs",
    actions: ["open_buddy", "dismiss"],
  },
  {
    producer: "engine/src/buddy/jobs/quest_prompt.rs",
    actions: ["accept_quest", "dismiss"],
  },
  {
    producer: "engine/src/buddy/jobs/proactive_suggestions.rs",
    actions: [
      "review_worktree_cleanup",
      "create_worktrees_pulse",
      "dismiss_suggestion",
    ],
  },
  {
    producer: "engine/crates/refact-buddy-core/src/state.rs",
    actions: ["open_setup", "care_play", "open_buddy", "dismiss"],
  },
  {
    producer: "engine/src/ext/competitor_import/mod.rs",
    actions: ["open_buddy", "dismiss"],
  },
  {
    producer: "engine/src/tools/tool_say.rs",
    actions: [
      "open_setup",
      "open_setup_mode",
      "open_stats",
      "open_buddy",
      "dismiss",
      "open_setup_mcp",
      "open_setup_skills",
      "open_setup_commands",
      "open_setup_agents_md",
      "open_setup_subagents",
    ],
  },
] as const;

const LOCAL_OR_TYPED_CONTROL_PREFIXES = [
  "local_fetch",
  "local_dismiss",
  "opportunity_action:",
] as const;

const allProductionActions = [
  ...new Set(DIRECT_PRODUCTION_CONTROLS.flatMap(({ actions }) => actions)),
];

function control(action: string, action_param?: string): BuddyControl {
  return {
    id: `production-${action}`,
    label: action,
    action,
    action_param,
    style: "primary",
  };
}

function makeDispatch() {
  const dispatch = vi.fn((action: unknown) => action);
  return dispatch as never;
}

describe("operator compatibility authority", () => {
  it.each(["enabled", "unavailable"])(
    "blocks legacy suggestion dismissal when %s without sending a second lifecycle request",
    async (mode) => {
      let writes = 0;
      server.use(
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json({
            schema_version: 1,
            mode,
            features: {
              storage: true,
              actions: false,
              supervision: false,
              improvement: false,
              editorial: false,
            },
            project: { project_id: "p", canonical_root: "/p" },
            worker: { worker_id: "w", engine_instance_id: "e", epoch: 1 },
          }),
        ),
        http.post("*/v1/buddy/suggestions/:id/dismiss", () => {
          writes += 1;
          return HttpResponse.json({});
        }),
      );
      const store = setUpStore({
        config: { lspPort: 8001, host: "vscode", themeProps: {} },
      });
      expect(
        await executeBuddyAction(
          control("dismiss_suggestion", "suggestion"),
          store.dispatch,
        ),
      ).toMatchObject({ status: "unsupported" });
      expect(writes).toBe(0);
    },
  );
  it("rejects unknown, inconsistent, and malformed authority rather than selecting legacy mode", () => {
    const disabled = {
      schema_version: 1,
      mode: "disabled",
      features: {
        storage: false,
        actions: false,
        supervision: false,
        improvement: false,
        editorial: false,
      },
      project: null,
      worker: null,
    };
    expect(parseOperatorStatus(disabled).mode).toBe("disabled");
    for (const value of [
      { ...disabled, mode: "future" },
      { ...disabled, mode: "enabled" },
      { ...disabled, schema_version: 2 },
      { ...disabled, worker: {} },
    ])
      expect(() => parseOperatorStatus(value)).toThrow();
  });
});

describe("executeBuddyAction production control contract", () => {
  it.each(["dismiss_suggestion", "create_worktrees_pulse"])(
    "blocks the shipped %s producer when operator owns work",
    async (action) => {
      const requests: string[] = [];
      server.use(
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json({
            schema_version: 1,
            mode: "enabled",
            features: {
              storage: true,
              actions: false,
              supervision: false,
              improvement: false,
              editorial: false,
            },
            project: { project_id: "fixture", canonical_root: "/fixture" },
            worker: {
              worker_id: "worker",
              engine_instance_id: "engine",
              epoch: 1,
            },
          }),
        ),
        http.post("*/v1/buddy/suggestions/:id/dismiss", ({ request }) => {
          requests.push(request.url);
          return HttpResponse.json({});
        }),
        http.post("*/v1/buddy/drafts/pulse_report", ({ request }) => {
          requests.push(request.url);
          return HttpResponse.json({});
        }),
      );
      const store = setUpStore({
        config: { lspPort: 8001, host: "vscode", themeProps: {} },
      });
      await expect(
        executeBuddyAction(
          control(action, "origin-suggestion"),
          store.dispatch,
        ),
      ).resolves.toMatchObject({ status: "unsupported" });
      expect(requests).toEqual([]);
      expect(store.getState().buddy.activeSpeech?.text).toContain(
        "current revision-bound actions",
      );
    },
  );
  it("keeps every direct production control in the executor contract", () => {
    expect(allProductionActions).toEqual(
      expect.arrayContaining([
        "open_buddy",
        "dismiss",
        "accept_quest",
        "review_worktree_cleanup",
        "create_worktrees_pulse",
        "dismiss_suggestion",
        "open_setup",
        "care_play",
        "open_setup_mode",
        "open_stats",
      ]),
    );
    expect(LOCAL_OR_TYPED_CONTROL_PREFIXES).toEqual([
      "local_fetch",
      "local_dismiss",
      "opportunity_action:",
    ]);
  });

  it.each([
    ["dismiss"],
    ["open_buddy"],
    ["open_stats"],
    ["open_setup"],
    ["open_setup_mode", "setup_mcp"],
    ["open_setup_mcp"],
    ["open_setup_skills"],
    ["open_setup_commands"],
    ["open_setup_agents_md"],
    ["open_setup_subagents"],
    ["review_worktree_cleanup"],
  ])(
    "executes the %s producer action",
    async (action, actionParam = undefined) => {
      const dispatch = makeDispatch();

      await expect(
        executeBuddyAction(control(action, actionParam), dispatch),
      ).resolves.toEqual({ status: "ok" });
      expect(dispatch).toHaveBeenCalled();
    },
  );

  it.each([
    ["dismiss_suggestion", undefined, "missing its suggestion id"],
    ["accept_quest", " ", "missing its quest id"],
    ["dismiss_runtime_event", undefined, "missing its event id"],
    ["not_a_production_control", undefined, "not available in this build"],
  ])(
    "returns unsupported feedback for malformed or unknown %s controls",
    async (action, actionParam, message) => {
      const dispatch = makeDispatch();

      await expect(
        executeBuddyAction(control(action, actionParam), dispatch),
      ).resolves.toEqual({
        status: "unsupported",
        message: expect.stringContaining(message),
      });
      expect(dispatch).toHaveBeenCalledWith(
        expect.objectContaining({ type: "buddy/setActiveSpeech" }),
      );
    },
  );

  it("does not silently resolve suggestion-preserving controls", () => {
    for (const action of ["dismiss", "dismiss_suggestion", "accept_quest"]) {
      expect(buddyActionResolvesSuggestion(action)).toBe(false);
    }
    expect(buddyActionResolvesSuggestion("open_buddy")).toBe(true);
  });
});
