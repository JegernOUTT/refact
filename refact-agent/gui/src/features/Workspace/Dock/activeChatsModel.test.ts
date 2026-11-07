import { describe, expect, it } from "vitest";

import type { ChatActivity } from "../../../services/refact/trajectories";
import {
  ACTIVE_CHAT_ATTENTION_MAX_AGE_MS,
  activeChatsNeedAttention,
  buildActiveChatEntries,
} from "./activeChatsModel";

const now = 1_800_000_000_000;
const activity = (overrides: Partial<ChatActivity>): ChatActivity => ({
  id: "chat",
  title: "Needs attention is just a title",
  updated_at_ms: now,
  session_state: null,
  recovery_state: "needs_action",
  schedule_count: 0,
  paused_schedule_count: 0,
  ...overrides,
});

describe("Active Chats attention age", () => {
  it("uses the newest record before filtering so older activity cannot reappear", () => {
    const stale = now - ACTIVE_CHAT_ATTENTION_MAX_AGE_MS - 1;
    const older = activity({
      updated_at_ms: stale - 1,
      session_state: "generating",
      recovery_state: null,
    });
    const latest = activity({ updated_at_ms: stale });
    expect(buildActiveChatEntries([older, latest], now)).toEqual([]);
    expect(buildActiveChatEntries([latest, older], now)).toEqual([]);
    expect(
      buildActiveChatEntries([older, activity({ recovery_state: null })], now),
    ).toEqual([]);
  });

  it("includes the exact 24h boundary and removes older, future and invalid attention", () => {
    const boundary = now - ACTIVE_CHAT_ATTENTION_MAX_AGE_MS;
    const entries = buildActiveChatEntries(
      [
        activity({ id: "boundary", updated_at_ms: boundary }),
        activity({ id: "recent", updated_at_ms: boundary + 1 }),
        activity({ id: "old", updated_at_ms: boundary - 1 }),
        activity({ id: "future", updated_at_ms: now + 1 }),
        activity({ id: "unknown", updated_at_ms: 0 }),
        activity({ id: "invalid", updated_at_ms: NaN }),
        activity({ id: "infinite", updated_at_ms: Infinity }),
        activity({ id: "negative", updated_at_ms: -1 }),
      ],
      now,
    );
    expect(entries.map((entry) => entry.id)).toEqual(["recent", "boundary"]);
    expect(activeChatsNeedAttention(entries)).toBe(true);
    expect(
      buildActiveChatEntries([activity({ updated_at_ms: boundary })], now + 1),
    ).toEqual([]);
  });

  it("removes only stale attention reasons without hiding other active states or using titles", () => {
    const stale = now - ACTIVE_CHAT_ATTENTION_MAX_AGE_MS - 1;
    const entries = buildActiveChatEntries(
      [
        activity({
          id: "working",
          updated_at_ms: stale,
          session_state: "executing_tools",
        }),
        activity({
          id: "streaming",
          updated_at_ms: stale,
          session_state: "generating",
        }),
        activity({
          id: "recovering",
          updated_at_ms: stale,
          recovery_state: "waiting",
        }),
        activity({
          id: "scheduled",
          updated_at_ms: stale,
          schedule_count: 2,
          paused_schedule_count: 1,
        }),
        activity({ id: "title-only", recovery_state: null }),
      ],
      now,
    );
    expect(
      Object.fromEntries(entries.map((entry) => [entry.id, entry.reasons])),
    ).toEqual({
      working: ["working"],
      streaming: ["streaming"],
      recovering: ["recovering"],
      scheduled: ["scheduled", "schedule_paused"],
    });
    expect(activeChatsNeedAttention(entries)).toBe(false);
  });
});
