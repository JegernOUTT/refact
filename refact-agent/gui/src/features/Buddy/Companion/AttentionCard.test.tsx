import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, fireEvent, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { AttentionCard, type AttentionMemory } from "./AttentionCard";
import type {
  AttentionRow,
  AttentionSnapshot,
} from "../../../services/refact/buddyCompanion";

const row: AttentionRow = {
  id: "request-a",
  kind: "tool_permission",
  chat_id: "source-a",
  source_version: "v1",
  reason: "Permission required",
  effect: "execute_pending_tool",
  actionable: true,
  literal_command: "echo hello",
  display_command: "shell echo hello",
  cwd: null,
  tool_call_id: "tool-a",
  question: null,
  parent_chat_id: null,
  run_generation: null,
  expires_at: null,
  tool_name: "shell",
  tool_arguments: { command: "echo hello" },
};
const snapshot: AttentionSnapshot = {
  schema_version: 1,
  project_root: "/project",
  engine_identity: "engine",
  coverage: "loaded_sessions",
  partial: true,
  busy_chat_ids: [],
  rows: [row],
};
const config = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};
function memory(): AttentionMemory {
  return { drafts: {}, comments: "", locked: false };
}
describe("Buddy attention actions", () => {
  it("submits once despite duplicate clicks and retains uncertain intent across remount", async () => {
    const posts: unknown[] = [];
    server.use(
      http.post(
        "*/v1/buddy/approvals/source-a/response",
        async ({ request }) => {
          posts.push(await request.json());
          return new HttpResponse(null, { status: 503 });
        },
      ),
      http.get(
        "*/v1/buddy/approvals/source-a/outcomes/:id",
        () => new HttpResponse(null, { status: 503 }),
      ),
    );
    const state = memory();
    const changed = (patch: Partial<AttentionMemory>) =>
      Object.assign(state, patch);
    const props = {
      row,
      snapshot,
      fresh: true,
      memory: state,
      onMemory: changed,
      onNavigate: vi.fn(),
      onAsk: vi.fn(),
      onNext: vi.fn(),
    };
    const first = render(<AttentionCard {...props} />, {
      preloadedState: config,
    });
    const approve = screen.getByRole("button", { name: "Approve once" });
    fireEvent.click(approve);
    fireEvent.click(approve);
    await waitFor(() => expect(posts).toHaveLength(1));
    expect(state.intent?.id).toBeTruthy();
    first.unmount();
    render(<AttentionCard {...props} />, { preloadedState: config });
    expect(screen.getByRole("button", { name: "Approve once" })).toBeDisabled();
    expect(posts).toHaveLength(1);
    expect(state.locked).toBe(true);
  });
  it("disables an old selected card after a new source version without replacing its command", () => {
    const changedSnapshot = {
      ...snapshot,
      rows: [{ ...row, source_version: "v2", literal_command: "rm new" }],
    };
    render(
      <AttentionCard
        row={row}
        snapshot={changedSnapshot}
        fresh
        memory={memory()}
        onMemory={vi.fn()}
        onNavigate={vi.fn()}
        onAsk={vi.fn()}
        onNext={vi.fn()}
      />,
      { preloadedState: config },
    );
    expect(screen.getByRole("button", { name: "Approve once" })).toBeDisabled();
    expect(screen.getByText("echo hello")).toBeInTheDocument();
    expect(screen.queryByText("rm new")).toBeNull();
  });
  it("does not focus an incoming card and unknown specialized effects have no generic approve", () => {
    const input = document.createElement("textarea");
    document.body.append(input);
    input.focus();
    const specialized = { ...row, effect: "cache_extra_cost" };
    render(
      <AttentionCard
        row={specialized}
        snapshot={{ ...snapshot, rows: [specialized] }}
        fresh
        memory={memory()}
        onMemory={vi.fn()}
        onNavigate={vi.fn()}
        onAsk={vi.fn()}
        onNext={vi.fn()}
      />,
      { preloadedState: config },
    );
    expect(document.activeElement).toBe(input);
    expect(screen.queryByRole("button", { name: "Approve once" })).toBeNull();
    expect(screen.getByRole("button", { name: "Go to chat" })).toBeEnabled();
    input.remove();
  });
  it("does not display applied success from a receipt for another tool target", async () => {
    server.use(
      http.get("*/v1/buddy/approvals/source-a/outcomes/:id", () =>
        HttpResponse.json({
          project_root: "/project",
          engine_identity: "engine",
          chat_id: "source-a",
          client_request_id: "intent-a",
          request_identity: "separate-namespace",
          source_version: "v1",
          payload_hash: "hash",
          target_id: "different-tool",
          accepted: true,
          status: "applied",
          followup: "not_requested",
        }),
      ),
    );
    const state = memory();
    state.locked = true;
    state.intent = { id: "intent-a", accepted: true, engine: "engine" };
    render(
      <AttentionCard
        row={row}
        snapshot={snapshot}
        fresh
        memory={state}
        onMemory={(patch) => Object.assign(state, patch)}
        onNavigate={vi.fn()}
        onAsk={vi.fn()}
        onNext={vi.fn()}
      />,
      { preloadedState: config },
    );
    await waitFor(() =>
      expect(state.error).toContain("Receipt identity changed"),
    );
    expect(state.receipt).toBeUndefined();
    expect(screen.queryByText(/Decision: Applied/)).toBeNull();
  });
});
