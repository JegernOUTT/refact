import { describe, expect, it } from "vitest";
import {
  BUDDY_FAILURE_LABELS,
  BUDDY_RULE_LABELS,
  BUDDY_TRAIT_LABELS,
  BUDDY_WORKFLOW_LABELS,
  buddyFailureLabel,
  buddyRuleLabel,
  buddyWorkflowLabel,
  buddyTraitLabel,
  distinctBuddyLabels,
  humanizeBuddyIdentifier,
  humanizeBuddyKey,
} from "../buddyLabels";

describe("buddy label maps", () => {
  it("maps known failure codes", () => {
    expect(buddyFailureLabel("llm_error")).toBe("Model error");
    expect(buddyFailureLabel("tool_error")).toBe("Tool error");
    expect(buddyFailureLabel("browser_error")).toBe("Browser error");
    expect(buddyFailureLabel("network_error")).toBe("Network error");
    expect(buddyFailureLabel("generic")).toBe("Other");
    expect(buddyFailureLabel("LLM_ERROR")).toBe("Model error");
    expect(buddyFailureLabel(" ")).toBe("Other");
    expect(buddyFailureLabel(null)).toBe("Other");
  });

  it("maps workflow keys", () => {
    expect(buddyWorkflowLabel("buddy_daily_digest")).toBe("Daily digest");
    expect(buddyWorkflowLabel("kg_enrich")).toBe("Knowledge enrichment");
    expect(buddyWorkflowLabel(undefined)).toBe("Background work");
  });

  it("maps muted rule keys, including full cooldown keys", () => {
    expect(buddyRuleLabel("task_health:abandoned")).toBe("Abandoned tasks");
    expect(buddyRuleLabel("task_health:stuck:42")).toBe("Stuck tasks");
    expect(buddyRuleLabel("buddy_memory_garden")).toBe("Memory garden");
    expect(buddyRuleLabel("")).toBe("Saved rule");
  });

  it("has sentence-case, non-empty labels everywhere", () => {
    for (const map of [
      BUDDY_FAILURE_LABELS,
      BUDDY_WORKFLOW_LABELS,
      BUDDY_RULE_LABELS,
      BUDDY_TRAIT_LABELS,
    ]) {
      for (const value of Object.values(map)) {
        const label = value ?? "";
        expect(label.trim()).not.toBe("");
        expect(label.charAt(0)).toBe(label.charAt(0).toUpperCase());
        expect(label).not.toMatch(/_/);
      }
    }
  });

  it("humanizes unknown snake_case and colon keys", () => {
    expect(buddyFailureLabel("disk_full_error")).toBe("Disk full error");
    expect(humanizeBuddyKey("git_hygiene:large_diff")).toBe(
      "Git hygiene: large diff",
    );
    expect(
      humanizeBuddyKey("observer:3f2504e0-4f89-11d3-9a0c-0305e82c3301"),
    ).toBe("Observer");
    expect(buddyRuleLabel("weird_thing:deep_key")).toBe(
      "Weird thing: deep key",
    );
    expect(humanizeBuddyKey("")).toBe("Other");
  });
});

describe("humanizeBuddyIdentifier", () => {
  const uuid = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";

  it("shortens bare UUIDs", () => {
    expect(humanizeBuddyIdentifier(uuid)).toEqual({
      label: "3f2504e0",
      full: uuid,
    });
  });

  it("shortens refact:// URIs", () => {
    expect(humanizeBuddyIdentifier(`refact://chat/${uuid}`)).toEqual({
      label: "Chat 3f2504e0",
      full: `refact://chat/${uuid}`,
    });
    expect(humanizeBuddyIdentifier("refact://worktree/main").label).toBe(
      "Worktree main",
    );
    expect(humanizeBuddyIdentifier("refact://memory").label).toBe("Memory");
  });

  it("shortens prefixed chat ids", () => {
    expect(humanizeBuddyIdentifier(`chat-${uuid}`).label).toBe("Chat 3f2504e0");
    expect(humanizeBuddyIdentifier("buddy-check-source-" + uuid).label).toBe(
      "Buddy check source 3f2504e0",
    );
    expect(humanizeBuddyIdentifier("run_0123456789abcdef0123").label).toBe(
      "Run 01234567",
    );
  });

  it("keeps the distinguishing part of prefixed opaque ids", () => {
    const first = humanizeBuddyIdentifier(
      "refact://memory-lifecycle/memcand_traj_08c0024ad85a896ccd77e",
    ).label;
    const second = humanizeBuddyIdentifier(
      "refact://memory-lifecycle/memcand_traj_0b0ca154c33943dec1de7",
    ).label;
    expect(first).toBe("Memory lifecycle 08c0024a");
    expect(second).toBe("Memory lifecycle 0b0ca154");
    expect(first).not.toContain("memcand");
  });

  it("keeps readable values and handles empty input", () => {
    expect(humanizeBuddyIdentifier("main")).toEqual({
      label: "main",
      full: "main",
    });
    expect(humanizeBuddyIdentifier(null)).toEqual({ label: "", full: "" });
  });
});

describe("distinctBuddyLabels", () => {
  it("numbers labels that would otherwise collide", () => {
    expect(distinctBuddyLabels(["Memory", "Chat", "Memory"])).toEqual([
      "Memory #1",
      "Chat",
      "Memory #2",
    ]);
    expect(distinctBuddyLabels([])).toEqual([]);
  });
});

describe("buddyTraitLabel", () => {
  it("shows sentence-case trait names", () => {
    expect(buddyTraitLabel("playfulness")).toBe("Playfulness");
    expect(buddyTraitLabel("sociability")).toBe("Sociability");
    expect(buddyTraitLabel("night_owl")).toBe("Night owl");
    expect(buddyTraitLabel("")).toBe("Trait");
  });
});
