import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { AutonomousChats } from "../AutonomousChats";
import { normalizeConversationsPayload } from "../../../services/refact/buddy";
import type { BuddyConversationEntry, BuddyLedgerDiagnostics } from "../types";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

function makeWorkflowEntry(
  overrides?: Partial<BuddyConversationEntry>,
): BuddyConversationEntry {
  return {
    id: "refact_error_detective",
    kind: "workflow",
    title: "refact error detective (Error Detective)",
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T01:00:00Z",
    status: "completed",
    message_count: 4,
    icon: "🕵️",
    badge: "Error Detective",
    ...overrides,
  };
}

function makeDiagnostics(
  overrides?: Partial<BuddyLedgerDiagnostics>,
): BuddyLedgerDiagnostics {
  return {
    invalid_json: 0,
    missing_id: 0,
    repaired_id_alias: 0,
    empty_conversation: 0,
    quarantined: 0,
    ...overrides,
  };
}

describe("normalizeConversationsPayload", () => {
  it("accepts the envelope shape with diagnostics", () => {
    const entry = makeWorkflowEntry();
    const diagnostics = makeDiagnostics({ invalid_json: 2, quarantined: 2 });
    const result = normalizeConversationsPayload({
      entries: [entry],
      diagnostics,
    });
    expect(result.entries).toEqual([entry]);
    expect(result.diagnostics).toEqual(diagnostics);
  });

  it("accepts the legacy bare-array shape", () => {
    const entry = makeWorkflowEntry();
    const result = normalizeConversationsPayload([entry]);
    expect(result.entries).toEqual([entry]);
    expect(result.diagnostics).toBeNull();
  });

  it("falls back to empty on garbage payloads", () => {
    expect(normalizeConversationsPayload(null)).toEqual({
      entries: [],
      diagnostics: null,
    });
    expect(normalizeConversationsPayload({ nope: true })).toEqual({
      entries: [],
      diagnostics: null,
    });
  });
});

describe("AutonomousChats", () => {
  it("renders workflow groups and the corruption chip from diagnostics", async () => {
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json({
          entries: [makeWorkflowEntry()],
          diagnostics: makeDiagnostics({
            invalid_json: 1,
            missing_id: 1,
            quarantined: 2,
          }),
        }),
      ),
    );

    render(<AutonomousChats />, { preloadedState: CONFIG_STATE });

    expect(await screen.findByText("Error Detective")).toBeInTheDocument();
    const chip = await screen.findByTestId("ledger-corruption-chip");
    expect(chip.textContent).toContain("4 corrupt files quarantined");
  });

  it("renders without a chip for legacy array responses", async () => {
    server.use(
      http.get("*/v1/buddy/conversations", () =>
        HttpResponse.json([makeWorkflowEntry()]),
      ),
    );

    render(<AutonomousChats />, { preloadedState: CONFIG_STATE });

    expect(await screen.findByText("Error Detective")).toBeInTheDocument();
    expect(screen.queryByTestId("ledger-corruption-chip")).toBeNull();
  });
});
