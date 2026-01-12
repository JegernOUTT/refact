import { describe, it, expect, beforeEach } from "vitest";
import {
  saveLastThreadParams,
  getLastThreadParams,
  clearLastThreadParams,
  saveDraftMessage,
  getDraftMessage,
  clearDraftMessage,
  clearAllDraftMessages,
  pruneStaleDraftMessages,
} from "../threadStorage";

describe("threadStorage", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  describe("thread parameters", () => {
    it("should save and retrieve thread params", () => {
      const params = {
        model: "gpt-4",
        tool_use: "agent" as const,
        boost_reasoning: true,
      };

      saveLastThreadParams(params);
      const retrieved = getLastThreadParams();

      expect(retrieved).toEqual(params);
    });

    it("should merge with existing params", () => {
      saveLastThreadParams({ model: "gpt-4", tool_use: "agent" as const });
      saveLastThreadParams({ boost_reasoning: true });

      const retrieved = getLastThreadParams();
      expect(retrieved).toEqual({
        model: "gpt-4",
        tool_use: "agent",
        boost_reasoning: true,
      });
    });

    it("should clear thread params", () => {
      saveLastThreadParams({ model: "gpt-4", tool_use: "agent" as const });
      clearLastThreadParams();

      const retrieved = getLastThreadParams();
      expect(retrieved).toEqual({});
    });
  });

  describe("draft messages", () => {
    it("should save and retrieve draft message", () => {
      const threadId = "thread-123";
      const content = "Hello, world!";

      saveDraftMessage(threadId, content);
      const retrieved = getDraftMessage(threadId);

      expect(retrieved).toBe(content);
    });

    it("should retrieve draft immediately after saving (simulating page refresh)", () => {
      const threadId = "thread-456";
      const content = "Draft before refresh";

      saveDraftMessage(threadId, content);

      const retrievedAfterRefresh = getDraftMessage(threadId);
      expect(retrievedAfterRefresh).toBe(content);
    });

    it("should return empty string for non-existent draft", () => {
      const retrieved = getDraftMessage("non-existent");
      expect(retrieved).toBe("");
    });

    it("should clear draft when content is empty", () => {
      saveDraftMessage("thread-123", "Some content");
      saveDraftMessage("thread-123", "");

      const retrieved = getDraftMessage("thread-123");
      expect(retrieved).toBe("");
    });

    it("should clear specific draft message", () => {
      saveDraftMessage("thread-1", "Content 1");
      saveDraftMessage("thread-2", "Content 2");

      clearDraftMessage("thread-1");

      expect(getDraftMessage("thread-1")).toBe("");
      expect(getDraftMessage("thread-2")).toBe("Content 2");
    });

    it("should clear all draft messages", () => {
      saveDraftMessage("thread-1", "Content 1");
      saveDraftMessage("thread-2", "Content 2");

      clearAllDraftMessages();

      expect(getDraftMessage("thread-1")).toBe("");
      expect(getDraftMessage("thread-2")).toBe("");
    });

    it("should prune stale drafts", () => {
      const now = Date.now();
      const eightDaysAgo = now - 8 * 24 * 60 * 60 * 1000;

      localStorage.setItem(
        "refact_draft_messages",
        JSON.stringify({
          "thread-old": { content: "Old content", timestamp: eightDaysAgo },
          "thread-new": { content: "New content", timestamp: now },
        }),
      );

      pruneStaleDraftMessages();

      expect(getDraftMessage("thread-old")).toBe("");
      expect(getDraftMessage("thread-new")).toBe("New content");
    });

    it("should limit draft messages to MAX_DRAFT_MESSAGES", () => {
      for (let i = 0; i < 60; i++) {
        saveDraftMessage(`thread-${i}`, `Content ${i}`);
      }

      const stored = JSON.parse(
        localStorage.getItem("refact_draft_messages") ?? "{}",
      ) as Record<string, unknown>;
      expect(Object.keys(stored).length).toBeLessThanOrEqual(50);
    });
  });
});
