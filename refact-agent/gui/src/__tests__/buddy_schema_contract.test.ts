import { describe, expect, test } from "vitest";
import type {
  BuddyAction,
  BuddyPage,
  BuddyRuntimeEvent,
} from "../features/Buddy/types";

function roundTrip<T>(value: T): unknown {
  return JSON.parse(JSON.stringify(value));
}

function assertBuddyPage(page: BuddyPage): string {
  switch (page.type) {
    case "buddy":
    case "stats":
    case "customization":
    case "providers":
    case "default_models":
    case "integrations":
    case "extensions":
    case "marketplace_hub":
    case "marketplace":
    case "skills_marketplace":
    case "commands_marketplace":
    case "delegates_marketplace":
    case "tasks_list":
    case "knowledge_graph":
    case "worktrees":
      return page.type;
    case "task_workspace":
      return page.task_id;
    case "setup_mode":
      return page.mode;
    default: {
      const _never: never = page;
      return _never;
    }
  }
}

function assertBuddyAction(action: BuddyAction): string {
  switch (action.kind) {
    case "open_page":
      return assertBuddyPage(action.page);
    case "launch_investigation_chat":
      return action.preload.initial_user_message;
    case "draft_skill":
    case "draft_command":
    case "draft_delegate":
    case "draft_mode":
      return action.draft_id + action.label;
    case "draft_agents_md_patch":
      return action.content;
    case "draft_defaults_change":
      return action.defaults_kind;
    case "draft_customization_change":
      return action.customization_kind + action.id;
    case "offer_marketplace_install":
      return action.market_kind + action.item_id;
    case "create_pulse_report":
      return action.scope;
    case "dismiss":
      return action.kind;
    default: {
      const _never: never = action;
      return _never;
    }
  }
}

describe("Buddy schema contract", () => {
  test("BuddyPage fixtures round-trip through canonical discriminants", () => {
    const pages: BuddyPage[] = [
      { type: "buddy" },
      { type: "stats" },
      { type: "customization" },
      { type: "providers" },
      { type: "default_models" },
      { type: "integrations" },
      { type: "extensions" },
      { type: "marketplace_hub" },
      { type: "marketplace" },
      { type: "skills_marketplace" },
      { type: "commands_marketplace" },
      { type: "delegates_marketplace" },
      { type: "tasks_list" },
      { type: "task_workspace", task_id: "task-1" },
      { type: "knowledge_graph" },
      { type: "worktrees" },
      { type: "setup_mode", mode: "setup_mcp" },
    ];

    for (const page of pages) {
      const parsed = roundTrip(page) as BuddyPage;
      const expected =
        page.type === "task_workspace"
          ? page.task_id
          : page.type === "setup_mode"
            ? page.mode
            : page.type;
      expect(assertBuddyPage(parsed)).toBe(expected);
    }
  });

  test("BuddyAction fixtures round-trip through canonical discriminants", () => {
    const actions: BuddyAction[] = [
      { kind: "open_page", page: { type: "marketplace" } },
      {
        kind: "launch_investigation_chat",
        preload: {
          fact_keys: [],
          diagnostic_ids: [],
          log_excerpt: "",
          config_summary: "",
          initial_user_message: "Investigate",
        },
      },
      { kind: "draft_skill", draft_id: "d1", label: "Skill" },
      { kind: "draft_command", draft_id: "d2", label: "Command" },
      { kind: "draft_delegate", draft_id: "d3", label: "Delegate" },
      { kind: "draft_mode", draft_id: "d4", label: "Mode" },
      { kind: "draft_agents_md_patch", content: "# AGENTS.md" },
      {
        kind: "draft_defaults_change",
        defaults_kind: "chat_model",
        patch: {},
      },
      {
        kind: "draft_customization_change",
        customization_kind: "delegate",
        id: "delegate-1",
        patch: {},
      },
      {
        kind: "offer_marketplace_install",
        market_kind: "delegate",
        item_id: "item-1",
      },
      { kind: "create_pulse_report", scope: "all" },
      { kind: "dismiss" },
    ];

    for (const action of actions) {
      const parsed = roundTrip(action) as BuddyAction;
      expect(parsed.kind).toBe(action.kind);
      expect(assertBuddyAction(parsed)).toBeTruthy();
    }
  });

  test("OpenPage action has no params field", () => {
    const action: BuddyAction = { kind: "open_page", page: { type: "buddy" } };
    const parsed = roundTrip(action) as Record<string, unknown>;
    expect(parsed.params).toBeUndefined();
  });

  test("runtime event fixture accepts backend null option fields", () => {
    const event = roundTrip({
      id: "runtime-1",
      signal_type: "indexing",
      title: "Indexing",
      description: null,
      source: "indexer",
      status: "started",
      progress: null,
      dedupe_key: null,
      priority: "normal",
      created_at: "2024-01-01T00:00:00Z",
      ttl_ms: null,
      speech_text: null,
      scene: null,
      duration_hint: null,
      persistent: false,
      controls: [],
      dismissed: false,
    }) as BuddyRuntimeEvent;

    expect(event.description).toBeNull();
    expect(event.progress).toBeNull();
    expect(event.dedupe_key).toBeNull();
    expect(event.ttl_ms).toBeNull();
    expect(event.speech_text).toBeNull();
    expect(event.scene).toBeNull();
    expect(event.duration_hint).toBeNull();
  });
});
