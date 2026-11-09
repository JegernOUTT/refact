import { describe, expect, it } from "vitest";
import {
  appsSlice,
  appContextStaged,
  appContextsConsumed,
  type StagedAppContext,
} from "../../features/Apps/appsSlice";
import {
  APP_CONTEXT_MAX_PER_MESSAGE,
  appContextKey,
  inFlightAppContextKeys,
  isTicketId,
  markAppContextsInFlight,
  planAppContexts,
  selectStagedAppContextsById,
  sentAppContextIds,
} from "./appContext";

const TICKET = "0b9a3f5e-1c2d-4e6f-8a7b-9c0d1e2f3a4b";

function ticket(index: number): string {
  return `0b9a3f5e-1c2d-4e6f-8a7b-${String(index).padStart(12, "0")}`;
}

function staged(patch: Partial<StagedAppContext> = {}): StagedAppContext {
  return {
    id: TICKET,
    app_id: "board",
    binding_id: "binding-1",
    revision: "r1",
    title: "Card",
    text: "Selected card",
    ...patch,
  };
}

describe("app context planning", () => {
  it("freezes presentation attribution from typed top-level fields only", () => {
    const { included } = planAppContexts([
      staged({
        view_id: "main",
        captured_at: "2025-01-01T00:00:00Z",
        source_refs: [{ path: "notes/a.md" }, { path: "notes/a.md" }],
        data: { source_refs: ["forged/by-data.md"], label: "forged" },
      }),
    ]);
    expect(included).toHaveLength(1);
    expect(included[0]).toMatchObject({
      id: TICKET,
      app_id: "board",
      binding_id: "binding-1",
      view_id: "main",
      revision: "r1",
      label: "Card",
      captured_at: "2025-01-01T00:00:00Z",
      source_refs: ["notes/a.md"],
    });
  });

  it("collapses control characters in app-claimed labels and refs", () => {
    const { included } = planAppContexts([
      staged({
        title: "Line one\n[END]",
        source_refs: [{ path: "a\nb" }],
      }),
    ]);
    expect(included[0]?.label).toBe("Line one [END]");
    expect(included[0]?.source_refs).toEqual(["a b"]);
  });

  it("accepts only host-minted UUID tickets", () => {
    expect(isTicketId(TICKET)).toBe(true);
    expect(isTicketId("selection")).toBe(false);
    const plan = planAppContexts([staged({ id: "selection" }), staged()]);
    expect(plan.included.map((item) => item.id)).toEqual([TICKET]);
    expect(plan.deferred).toEqual([
      expect.objectContaining({ reason: "invalid_ticket" }),
    ]);
  });

  it("sends the first eight tickets and preserves the rest as deferred", () => {
    const many = Array.from(
      { length: APP_CONTEXT_MAX_PER_MESSAGE + 2 },
      (_, i) => staged({ id: ticket(i) }),
    );
    const plan = planAppContexts(many);
    expect(plan.included.map((item) => item.id)).toEqual(
      many.slice(0, APP_CONTEXT_MAX_PER_MESSAGE).map((item) => item.id),
    );
    expect(plan.deferred.map((item) => item.reason)).toEqual([
      "limit",
      "limit",
    ]);
  });

  it("skips tickets whose exact revision is already in flight", () => {
    const first = planAppContexts([staged()], undefined, "chat").included;
    const release = markAppContextsInFlight("chat", first);
    try {
      const again = planAppContexts(
        [staged(), staged({ id: ticket(2) })],
        inFlightAppContextKeys(),
        "chat",
      );
      expect(again.included.map((item) => item.id)).toEqual([ticket(2)]);
      expect(
        inFlightAppContextKeys().has(appContextKey("chat", TICKET, "r1")),
      ).toBe(true);
    } finally {
      release();
    }
    expect(inFlightAppContextKeys().size).toBe(0);
  });
});

describe("app context consumption with the real reducer", () => {
  it("consumes only exact sent tickets, keeping other chats and later stages", () => {
    const sent = planAppContexts([staged()]).included;
    let state = appsSlice.reducer(
      undefined,
      appContextStaged({ chatId: "owner", context: staged() }),
    );
    state = appsSlice.reducer(
      state,
      appContextStaged({ chatId: "other", context: staged() }),
    );
    state = appsSlice.reducer(
      state,
      appContextStaged({
        chatId: "owner",
        context: staged({ id: ticket(3), text: "added during send" }),
      }),
    );

    const ids = sentAppContextIds(
      selectStagedAppContextsById({ apps: state }, "owner"),
      sent,
    );
    expect(ids).toEqual([TICKET]);

    const after = appsSlice.reducer(
      state,
      appContextsConsumed({ chatId: "owner", ids }),
    );
    expect(after.stagedByChat.owner?.map((item) => item.id)).toEqual([
      ticket(3),
    ]);
    expect(after.stagedByChat.other).toHaveLength(1);
  });

  it("does not consume a ticket that was restaged with different content", () => {
    const sent = planAppContexts([staged()]).included;
    const restaged = appsSlice.reducer(
      undefined,
      appContextStaged({
        chatId: "owner",
        context: staged({ revision: "r2", text: "newer body" }),
      }),
    );
    expect(
      sentAppContextIds(
        selectStagedAppContextsById({ apps: restaged }, "owner"),
        sent,
      ),
    ).toEqual([]);
  });
});
