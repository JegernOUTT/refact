import { http, HttpResponse } from "msw";
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyNativeWork } from "./BuddyNativeWork";
import { parseNativeWork } from "../../services/refact/buddyNativeWork";

const session = {
  chat_id: "native",
  title: "Provider reliability",
  model: "chosen/model",
  state: "paused",
  persistence_pending: true,
  agents: [
    {
      agent_id: "child",
      title: "Provider check",
      status: "completed",
      model: "chosen/model",
      progress: null,
    },
  ],
  agents_truncated: false,
  pending_questions: 1,
  approval_count: 2,
  browser_decision_pending: false,
  error: null,
  goal: {
    version: 1,
    content: "Keep the selected model",
    active: true,
    status: "paused",
    turns_used: 2,
    tokens_used: 300,
    max_turns: null,
    max_tokens: null,
    latest_verdict: null,
    gaps: [],
    verification_blocked: true,
    truncated: false,
  },
};
const snapshot = {
  schema_version: 1,
  project_root: "/scratch",
  coverage: "loaded_native_sessions",
  next_cursor: null,
  sessions: [session],
};
const config = {
  config: { host: "vscode" as const, lspPort: 8001, themeProps: {} },
};

describe("Native work status", () => {
  it("loads on demand and keeps worker completion separate from goal verification", async () => {
    let reads = 0;
    server.use(
      http.get("*/v1/buddy/native-work", () => {
        reads += 1;
        return HttpResponse.json(snapshot);
      }),
    );
    render(<BuddyNativeWork />, { preloadedState: config });
    expect(reads).toBe(0);
    const details = screen
      .getByText("Live chats, goals and workers")
      .closest("details");
    if (!details) throw new Error("native work disclosure missing");
    details.open = true;
    fireEvent(details, new Event("toggle"));
    expect(await screen.findByText("Provider reliability")).toBeTruthy();
    expect(screen.getByText("Keep the selected model")).toBeTruthy();
    expect(
      screen.getByText(/Worker completion is not verification/),
    ).toBeTruthy();
    expect(screen.getByText(/awaiting persistence/)).toBeTruthy();
    expect(screen.getByText(/2 tool or privacy approvals/)).toBeTruthy();
    expect(screen.getByText(/Verification is blocked/)).toBeTruthy();
    await waitFor(() => expect(reads).toBe(1));
  });

  it("rejects unsupported states, unsafe counts and duplicate native identities", () => {
    expect(parseNativeWork(snapshot).sessions[0].goal?.status).toBe("paused");
    for (const value of [
      { ...snapshot, schema_version: 2 },
      { ...snapshot, sessions: [session, session] },
      { ...snapshot, sessions: [{ ...session, state: "made_up" }] },
      { ...snapshot, sessions: [{ ...session, pending_questions: -1 }] },
      {
        ...snapshot,
        sessions: [
          {
            ...session,
            goal: { ...session.goal, tokens_used: Number.MAX_SAFE_INTEGER + 1 },
          },
        ],
      },
    ])
      expect(() => parseNativeWork(value)).toThrow();
  });
});
