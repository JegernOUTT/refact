import { http, HttpResponse } from "msw";
import { describe, it, expect, vi } from "vitest";
import {
  act,
  render,
  screen,
  fireEvent,
  waitFor,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyNativeDecisions } from "./BuddyNativeDecisions";
import { parseNativeDecisionSnapshot } from "../../services/refact/buddyDecisions";

const question = {
  tool_call_id: "ask-1",
  asked_at_ms: 1,
  status: "pending",
  questions: [{ id: "continue", type: "yes_no", text: "Continue the check?" }],
};
const row = { project_root: "/scratch", chat_id: "chat-a", question };
const snapshot = {
  schema_version: 1,
  project_root: "/scratch",
  coverage: "loaded_native_sessions",
  truncated: false,
  questions: [row],
};
const config = {
  config: { host: "vscode" as const, lspPort: 8001, themeProps: {} },
};

describe("Buddy native decisions", () => {
  it("rejects unknown versions, invalid choices and duplicate question IDs", () => {
    expect(() =>
      parseNativeDecisionSnapshot({ ...snapshot, schema_version: 2 }),
    ).toThrow();
    for (const questions of [
      [{ id: "x", text: "Pick", type: "single_select" }],
      [question.questions[0], question.questions[0]],
      [{ id: "x", text: "Pick", type: "execute_script" }],
    ])
      expect(() =>
        parseNativeDecisionSnapshot({
          ...snapshot,
          questions: [{ ...row, question: { ...question, questions } }],
        }),
      ).toThrow();
  });

  it("sends a native answer once and waits for native resolution rather than HTTP queue acceptance", async () => {
    let state = "pending";
    const commands: Record<string, unknown>[] = [];
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json({ ...row, question: { ...question, status: state } }),
      ),
      http.post(
        "*/v1/buddy/decisions/questions/chat-a/ask-1",
        async ({ request }) => {
          commands.push((await request.json()) as Record<string, unknown>);
          return HttpResponse.json({ status: "queued" });
        },
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    const choice = await screen.findByRole("radio", { name: "Yes" });
    await waitFor(() => expect(choice).toBeEnabled());
    fireEvent.click(choice);
    const submit = screen.getByRole("button", { name: "Submit answer" });
    fireEvent.click(submit);
    fireEvent.click(submit);
    expect(await screen.findByText(/Answer queued/)).toBeTruthy();
    expect(commands).toHaveLength(1);
    expect(commands[0]).toMatchObject({
      project_root: "/scratch",
      asked_at_ms: 1,
      answers: [{ id: "continue", answer: true }],
    });
    expect(screen.queryByText(/Answered — confirmed/)).toBeNull();
    state = "answered";
    fireEvent.click(
      screen.getByRole("button", { name: "Refresh decision status" }),
    );
    expect(await screen.findByText(/Answered — confirmed/)).toBeTruthy();
    expect(commands).toHaveLength(1);
  });

  it("blocks a stale card when another client cancelled the native question", async () => {
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json({
          ...row,
          question: { ...question, status: "cancelled" },
        }),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    expect(
      await screen.findByText(/Cancelled in the native chat/),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Submit answer" }),
    ).toBeDisabled();
    expect(screen.getByRole("radio", { name: "Yes" })).toBeDisabled();
  });

  it("shows a newly asked question without a manual refresh and keeps prior cards", async () => {
    let questions = [row];
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json({ ...snapshot, questions }),
      ),
      http.get("*/v1/buddy/decisions/questions/:chat/:tool", ({ params }) =>
        HttpResponse.json({
          ...row,
          chat_id: String(params.chat),
          question: { ...question, tool_call_id: String(params.tool) },
        }),
      ),
    );
    vi.useFakeTimers();
    const view = render(<BuddyNativeDecisions />, { preloadedState: config });
    try {
      await vi.waitFor(async () => {
        await act(async () => {
          await vi.advanceTimersByTimeAsync(50);
        });
        expect(screen.getByText(/Question from chat chat-a/)).toBeTruthy();
      });
      questions = [
        row,
        {
          ...row,
          chat_id: "chat-b",
          question: { ...question, tool_call_id: "ask-2" },
        },
      ];
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5000);
      });
      await vi.waitFor(async () => {
        await act(async () => {
          await vi.advanceTimersByTimeAsync(50);
        });
        expect(screen.getByText(/Question from chat chat-b/)).toBeTruthy();
      });
      expect(screen.getByText(/Question from chat chat-a/)).toBeTruthy();
    } finally {
      view.unmount();
      vi.useRealTimers();
    }
  });

  it("keeps an acknowledged card visible after the list drops it and never claims resolution", async () => {
    let listed = [row];
    let state = "pending";
    let lists = 0;
    server.use(
      http.get("*/v1/buddy/decisions/current", () => {
        lists++;
        return HttpResponse.json({ ...snapshot, questions: listed });
      }),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json({ ...row, question: { ...question, status: state } }),
      ),
      http.post("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json({ status: "queued" }),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    const choice = await screen.findByRole("radio", { name: "Yes" });
    await waitFor(() => expect(choice).toBeEnabled());
    fireEvent.click(choice);
    fireEvent.click(screen.getByRole("button", { name: "Submit answer" }));
    expect(await screen.findByText(/Answer queued/)).toBeTruthy();
    listed = [];
    const seen = lists;
    fireEvent.click(screen.getByRole("button", { name: "Refresh questions" }));
    await waitFor(() => expect(lists).toBeGreaterThan(seen));
    expect(screen.getByText(/Question from chat chat-a/)).toBeTruthy();
    expect(screen.queryByText(/No pending questions/)).toBeNull();
    expect(screen.queryByText(/Answered — confirmed/)).toBeNull();
    state = "answered";
    fireEvent.click(
      screen.getByRole("button", { name: "Refresh decision status" }),
    );
    expect(await screen.findByText(/Answered — confirmed/)).toBeTruthy();
  });

  it("keeps a typed draft across ordinary list updates for the same question", async () => {
    const freeText = {
      ...question,
      tool_call_id: "ask-text",
      questions: [{ id: "why", type: "free_text", text: "Why continue?" }],
    };
    const textRow = { ...row, question: freeText };
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json({ ...snapshot, questions: [textRow] }),
      ),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-text", () =>
        HttpResponse.json(textRow),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    const field = await screen.findByRole("textbox", {
      name: "Why continue?",
    });
    await waitFor(() => expect(field).toBeEnabled());
    fireEvent.change(field, { target: { value: "keep this draft" } });
    fireEvent.click(screen.getByRole("button", { name: "Refresh questions" }));
    await waitFor(() =>
      expect(
        screen.getByRole("textbox", { name: "Why continue?" }),
      ).toHaveValue("keep this draft"),
    );
  });

  it("does not confirm a different question instance as answered", async () => {
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json({
          ...row,
          question: { ...question, asked_at_ms: 2, status: "answered" },
        }),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    expect(await screen.findByText(/This question changed/)).toBeTruthy();
    expect(screen.queryByText(/Answered — confirmed/)).toBeNull();
    expect(
      screen.getByRole("button", { name: "Submit answer" }),
    ).toBeDisabled();
  });

  it("does not carry questions or answer drafts across a changed response root", async () => {
    let current = snapshot;
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json(current),
      ),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json(row),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    const choice = await screen.findByRole("radio", { name: "Yes" });
    await waitFor(() => expect(choice).toBeEnabled());
    fireEvent.click(choice);
    current = { ...snapshot, project_root: "/other", questions: [] };
    fireEvent.click(screen.getByRole("button", { name: "Refresh questions" }));
    await waitFor(() =>
      expect(screen.queryByText(/Question from chat chat-a/)).toBeNull(),
    );
    expect(screen.queryByRole("button", { name: "Submit answer" })).toBeNull();
  });

  it("renders nothing at all when there are no questions", async () => {
    let lists = 0;
    server.use(
      http.get("*/v1/buddy/decisions/current", () => {
        lists++;
        return HttpResponse.json({ ...snapshot, questions: [] });
      }),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    await waitFor(() => expect(lists).toBeGreaterThan(0));
    expect(screen.queryByLabelText("Native questions")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Refresh questions" }),
    ).toBeNull();
  });

  it("shows an error with a retry instead of an empty state when the request fails", async () => {
    let lists = 0;
    let offline = true;
    server.use(
      http.get("*/v1/buddy/decisions/current", () => {
        lists++;
        return offline
          ? HttpResponse.json({ error: "mode down" }, { status: 503 })
          : HttpResponse.json({ ...snapshot, questions: [] });
      }),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    expect(
      await screen.findByText(/Native questions could not be loaded/),
    ).toBeTruthy();
    expect(screen.queryByText(/No pending questions/)).toBeNull();
    const retry = screen.getByRole("button", { name: "Retry" });
    const seen = lists;
    offline = false;
    fireEvent.click(retry);
    await waitFor(() =>
      expect(
        screen.queryByText(/Native questions could not be loaded/),
      ).toBeNull(),
    );
    expect(lists).toBeGreaterThan(seen);
    expect(screen.queryByLabelText("Native questions")).toBeNull();
  });

  it("stops the repoll loop while the request keeps failing", async () => {
    let lists = 0;
    server.use(
      http.get("*/v1/buddy/decisions/current", () => {
        lists++;
        return HttpResponse.json({ error: "mode down" }, { status: 503 });
      }),
    );
    const view = render(<BuddyNativeDecisions />, { preloadedState: config });
    try {
      await screen.findByText(/Native questions could not be loaded/);
      const settled = lists;
      vi.useFakeTimers();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30000);
      });
      expect(lists).toBe(settled);
    } finally {
      vi.useRealTimers();
      view.unmount();
    }
  });
});
