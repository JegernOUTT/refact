import { http, HttpResponse } from "msw";
import { setupListeners } from "@reduxjs/toolkit/query";
import { afterEach, describe, it, expect, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyNativeDecisions } from "./BuddyNativeDecisions";
import { parseNativeDecisionSnapshot } from "../../services/refact/buddyDecisions";
import { setUpStore } from "../../app/store";

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
const workSession = {
  chat_id: "chat-a",
  title: "Finish multi-agent parity M1–M10",
  model: "chosen/model",
  state: "waiting_user_input",
  persistence_pending: false,
  goal: null,
  agents: [],
  agents_truncated: false,
  pending_questions: 1,
  approval_count: 0,
  browser_decision_pending: false,
  error: null,
};
const work = {
  schema_version: 1,
  project_root: "/scratch",
  coverage: "loaded_native_sessions",
  next_cursor: null,
  sessions: [workSession],
};
const config = {
  config: { host: "vscode" as const, lspPort: 8001, themeProps: {} },
};

const originalVisibilityState = Object.getOwnPropertyDescriptor(
  document,
  "visibilityState",
);
const originalHidden = Object.getOwnPropertyDescriptor(document, "hidden");

afterEach(() => {
  vi.useRealTimers();
  if (originalVisibilityState)
    Object.defineProperty(document, "visibilityState", originalVisibilityState);
  else Reflect.deleteProperty(document, "visibilityState");
  if (originalHidden) Object.defineProperty(document, "hidden", originalHidden);
  else Reflect.deleteProperty(document, "hidden");
});

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

  it("headlines a decision with the chat title and keeps the id as secondary detail", async () => {
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.get("*/v1/buddy/native-work", () => HttpResponse.json(work)),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json(row),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    expect(
      await screen.findByText("Finish multi-agent parity M1–M10"),
    ).toBeTruthy();
    expect(screen.queryByText("Question from an untitled chat")).toBeNull();
    expect(screen.queryByText("Chat chat-a")).toBeNull();
  });

  it("names an untitled chat and truncates its id instead of headlining a UUID", async () => {
    const chatId = "53ee2bb5-b29a-4488-a69e-b1b63113101e";
    const untitled = {
      ...row,
      chat_id: chatId,
      question: { ...question, tool_call_id: "ask-untitled" },
    };
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json({ ...snapshot, questions: [untitled] }),
      ),
      http.get("*/v1/buddy/native-work", () =>
        HttpResponse.json({
          ...work,
          sessions: [{ ...workSession, chat_id: chatId, title: "   " }],
        }),
      ),
      http.get("*/v1/buddy/decisions/questions/:chat/:tool", () =>
        HttpResponse.json(untitled),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    expect(
      await screen.findByText("Question from an untitled chat"),
    ).toBeTruthy();
    expect(screen.queryByText(chatId)).toBeNull();
    expect(screen.getByText("Chat 53ee2bb5")).toHaveAttribute("title", chatId);
  });

  it("does not borrow a chat title from another project root", async () => {
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.get("*/v1/buddy/native-work", () =>
        HttpResponse.json({ ...work, project_root: "/other" }),
      ),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json(row),
      ),
    );
    render(<BuddyNativeDecisions />, { preloadedState: config });
    expect(
      await screen.findByText("Question from an untitled chat"),
    ).toBeTruthy();
    expect(screen.queryByText("Finish multi-agent parity M1–M10")).toBeNull();
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
    expect(await screen.findByText(/Cancelled in the chat/)).toBeTruthy();
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
        expect(screen.getByText("Chat chat-a")).toBeTruthy();
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
        expect(screen.getByText("Chat chat-b")).toBeTruthy();
      });
      expect(screen.getByText("Chat chat-a")).toBeTruthy();
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
    expect(screen.getByText("Chat chat-a")).toBeTruthy();
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
    await waitFor(() => expect(screen.queryByText("Chat chat-a")).toBeNull());
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
    expect(screen.queryByTestId("buddy-home-questions")).toBeNull();
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
    expect(await screen.findByText("Couldn't load questions.")).toBeTruthy();
    expect(screen.queryByText(/No pending questions/)).toBeNull();
    const retry = screen.getByRole("button", { name: "Retry" });
    const seen = lists;
    offline = false;
    fireEvent.click(retry);
    await waitFor(() =>
      expect(screen.queryByText("Couldn't load questions.")).toBeNull(),
    );
    expect(lists).toBeGreaterThan(seen);
    expect(screen.queryByTestId("buddy-home-questions")).toBeNull();
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
      await screen.findByText("Couldn't load questions.");
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

  it("pauses decision polling while hidden and refetches on resume", async () => {
    let reads = 0;
    let visibility: DocumentVisibilityState = "visible";
    let hidden = false;
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => visibility,
    });
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => hidden,
    });
    server.use(
      http.get("*/v1/buddy/decisions/current", () => {
        reads += 1;
        return HttpResponse.json(snapshot);
      }),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () =>
        HttpResponse.json(row),
      ),
    );
    vi.useFakeTimers();
    const store = setUpStore(config);
    const stopListeners = setupListeners(store.dispatch);
    const view = render(<BuddyNativeDecisions />, { store });
    try {
      await vi.waitFor(async () => {
        await act(async () => {
          await vi.advanceTimersByTimeAsync(10);
        });
        expect(reads).toBe(1);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(12500);
      });
      const visibleReads = reads;
      expect(visibleReads).toBeGreaterThanOrEqual(3);

      act(() => {
        visibility = "hidden";
        hidden = true;
        document.dispatchEvent(new Event("visibilitychange"));
        window.dispatchEvent(new Event("visibilitychange"));
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      const hiddenReads = reads;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15000);
      });
      expect(reads).toBe(hiddenReads);
      expect(hiddenReads).toBeLessThanOrEqual(visibleReads + 1);

      act(() => {
        visibility = "visible";
        hidden = false;
        document.dispatchEvent(new Event("visibilitychange"));
        window.dispatchEvent(new Event("visibilitychange"));
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(50);
      });
      expect(reads).toBe(visibleReads + 1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5000);
      });
      expect(reads).toBe(visibleReads + 2);
    } finally {
      stopListeners();
      view.unmount();
    }
  });

  it("pauses question-status polling while hidden and refetches on resume", async () => {
    let statusReads = 0;
    let visibility: DocumentVisibilityState = "visible";
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => visibility,
    });
    server.use(
      http.get("*/v1/buddy/decisions/current", () =>
        HttpResponse.json(snapshot),
      ),
      http.get("*/v1/buddy/decisions/questions/chat-a/ask-1", () => {
        statusReads += 1;
        return HttpResponse.json(row);
      }),
    );
    vi.useFakeTimers();
    const store = setUpStore(config);
    const stopListeners = setupListeners(store.dispatch);
    const view = render(<BuddyNativeDecisions />, { store });
    try {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10);
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(6500);
      });
      const visibleReads = statusReads;
      expect(visibleReads).toBeGreaterThanOrEqual(3);

      act(() => {
        visibility = "hidden";
        document.dispatchEvent(new Event("visibilitychange"));
        window.dispatchEvent(new Event("visibilitychange"));
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(9000);
      });
      expect(statusReads).toBe(visibleReads);

      act(() => {
        visibility = "visible";
        document.dispatchEvent(new Event("visibilitychange"));
        window.dispatchEvent(new Event("visibilitychange"));
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(50);
      });
      expect(statusReads).toBe(visibleReads + 1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3000);
      });
      expect(statusReads).toBe(visibleReads + 2);
    } finally {
      stopListeners();
      view.unmount();
    }
  });
});
