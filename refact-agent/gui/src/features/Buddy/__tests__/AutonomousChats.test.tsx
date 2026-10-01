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
  it("uses the badge for the exact legacy title and preserves parenthetical titles", () => {
    render(
      <AutonomousChats
        conversations={[
          makeWorkflowEntry({
            id: "refact_error_detective",
            title: "refact error detective (Error Detective)",
            badge: "Error Detective",
          }),
          makeWorkflowEntry({
            id: "workflow-custom",
            title: "Fix login (urgent)",
            badge: "Fix Login",
          }),
        ]}
      />,
      { preloadedState: CONFIG_STATE },
    );

    expect(
      screen.getByRole("button", { name: "Open Error Detective" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open Fix login (urgent)" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("refact error detective (Error Detective)"),
    ).not.toBeInTheDocument();
  });

  it("renders capped semantic workflow lists with display names, plural counts, and duplicate rows", async () => {
    const entries = Array.from({ length: 60 }, (_, index) => {
      const workflow = Math.floor(index / 12);
      const ids = [
        "refact_error_detective",
        "buddy_pr_issue_matchmaker",
        "kg_deprecate",
        "privacy_observation_degraded",
        "commit_message",
      ];
      const names = [
        "Error Detective",
        "PR Matchmaker",
        "Knowledge",
        "Privacy observation degraded",
        "Commit message",
      ];
      const id = ids[workflow];
      const badge = names[workflow];
      return makeWorkflowEntry({
        id: index % 12 < 2 ? id : `${id}-${index}`,
        kind: "workflow",
        badge,
        title:
          index % 12 < 2
            ? `${id.replace(/_/g, " ")} (${badge})`
            : `${badge} report ${index}`,
        message_count: index === 0 ? 1 : 2,
      });
    });
    const { user } = render(<AutonomousChats conversations={entries} />, {
      preloadedState: CONFIG_STATE,
    });

    const itemCount = () =>
      screen.getAllByRole("button", { name: /^Open / }).length;
    expect(itemCount()).toBe(20);
    expect(screen.getAllByRole("heading", { level: 3 })).toHaveLength(2);
    expect(document.body.textContent).toContain("1 entry");
    expect(document.body.textContent).not.toMatch(/\b[a-z]+(?: [a-z]+){2,} \(/);
    expect(document.body.textContent).not.toMatch(
      /\b1 entries\b|\b1 messages\b/,
    );
    expect(document.body.textContent).toContain("2 chats");
    expect(itemCount()).toBeLessThanOrEqual(20);
    await user.click(screen.getByRole("button", { name: "Show more" }));
    expect(itemCount()).toBe(40);
  });

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

    expect(
      await screen.findByRole("heading", { name: "Error Detective" }),
    ).toBeInTheDocument();
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

    expect(
      await screen.findByRole("heading", { name: "Error Detective" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("ledger-corruption-chip")).toBeNull();
  });
});
