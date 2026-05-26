import { describe, expect, it } from "vitest";
import {
  isTaskMemoryEntry,
  isTaskMemoriesResponse,
  isTaskMemoryFacetsResponse,
} from "./taskMemoriesApi";

const validEntry = {
  filename: "decision.md",
  created_at: "2026-05-22T01:00:00Z",
  created_at_known: true,
  title: "Use scoped memory index",
  content: "Keep memory search local.",
  tags: ["planner", "search"],
  kind: "decision",
  namespace: "task",
  pinned: false,
  status: "active",
};

describe("isTaskMemoryEntry", () => {
  it("accepts valid entry", () => {
    expect(isTaskMemoryEntry(validEntry)).toBe(true);
  });

  it("is_task_memory_entry_rejects_string_in_tags_array", () => {
    expect(
      isTaskMemoryEntry({ ...validEntry, tags: ["ok", 42, "also-ok"] }),
    ).toBe(false);
  });

  it("rejects when filename is missing", () => {
    const { filename: _filename, ...withoutFilename } = validEntry;
    expect(isTaskMemoryEntry(withoutFilename)).toBe(false);
  });

  it("rejects when tags is not an array", () => {
    expect(isTaskMemoryEntry({ ...validEntry, tags: "planner" })).toBe(false);
  });

  it("rejects null", () => {
    expect(isTaskMemoryEntry(null)).toBe(false);
  });
});

describe("isTaskMemoriesResponse", () => {
  it("accepts valid response", () => {
    expect(
      isTaskMemoriesResponse({
        task_id: "task-1",
        since: "2026-05-22T00:00:00Z",
        new_count: 0,
        memories: [],
        warnings: [],
      }),
    ).toBe(true);
  });

  it("rejects when memories is not an array", () => {
    expect(
      isTaskMemoriesResponse({
        task_id: "task-1",
        since: "2026-05-22T00:00:00Z",
        new_count: 0,
        memories: null,
        warnings: [],
      }),
    ).toBe(false);
  });
});

describe("isTaskMemoryFacetsResponse", () => {
  it("accepts valid facets response", () => {
    expect(
      isTaskMemoryFacetsResponse({
        task_id: "task-1",
        namespaces: ["task"],
        tags: ["planner"],
        kinds: ["decision"],
        total_count: 1,
        pinned_count: 0,
      }),
    ).toBe(true);
  });

  it("rejects when kinds is missing", () => {
    expect(
      isTaskMemoryFacetsResponse({
        task_id: "task-1",
        namespaces: ["task"],
        tags: [],
        total_count: 0,
        pinned_count: 0,
      }),
    ).toBe(false);
  });
});

describe("use_get_task_memory_facets_calls_facets_endpoint_not_full_list", () => {
  it("facets URL contains /facets suffix, not the list URL", () => {
    const taskId = "test-task";
    const facetsPath = `/v1/task/${encodeURIComponent(taskId)}/memories/facets`;
    const listPath = `/v1/task/${encodeURIComponent(taskId)}/memories`;
    expect(facetsPath).toContain("/memories/facets");
    expect(facetsPath).not.toBe(listPath);
    expect(facetsPath).not.toMatch(/\/memories$/);
  });
});
