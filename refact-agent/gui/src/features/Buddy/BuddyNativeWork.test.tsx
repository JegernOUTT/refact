import { http, HttpResponse } from "msw";
import { setupListeners } from "@reduxjs/toolkit/query";
import { afterEach, beforeEach, describe, it, expect, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { render, screen, fireEvent, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddyNativeWork } from "./BuddyNativeWork";
import { resetBuddyPersistentState } from "./ui";
import { setUpStore } from "../../app/store";
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
  const originalVisibilityState = Object.getOwnPropertyDescriptor(
    document,
    "visibilityState",
  );
  let visibilityState: DocumentVisibilityState = "visible";

  beforeEach(() => {
    visibilityState = "visible";
    resetBuddyPersistentState();
  });

  afterEach(() => {
    vi.useRealTimers();
    if (originalVisibilityState)
      Object.defineProperty(
        document,
        "visibilityState",
        originalVisibilityState,
      );
    else Reflect.deleteProperty(document, "visibilityState");
  });

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
    const trigger = screen.getByRole("button", { name: /Background work/ });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(await screen.findByText("Provider reliability")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Goal and workers/ }));
    expect(screen.queryByText("native")).toBeNull();
    expect(screen.getByText("Paused")).toBeTruthy();
    expect(screen.getAllByText("model").length).toBeGreaterThan(0);
    expect(screen.getByText("Keep the selected model")).toBeTruthy();
    expect(
      screen.getByText(/Worker completion is not verification/),
    ).toBeTruthy();
    expect(screen.getByText(/awaiting persistence/)).toBeTruthy();
    expect(screen.getByText(/2 tool or privacy approvals/)).toBeTruthy();
    expect(screen.getByText(/Verification is blocked/)).toBeTruthy();
    await waitFor(() => expect(reads).toBe(1));
  });

  it("pauses native-work polling while hidden and refetches on resume", async () => {
    let reads = 0;
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => visibilityState,
    });
    server.use(
      http.get("*/v1/buddy/native-work", () => {
        reads += 1;
        return HttpResponse.json(snapshot);
      }),
    );
    vi.useFakeTimers();
    const store = setUpStore(config);
    const stopListeners = setupListeners(store.dispatch);
    const view = render(<BuddyNativeWork />, { store });
    fireEvent.click(screen.getByRole("button", { name: /Background work/ }));
    await vi.waitFor(async () => {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10);
      });
      expect(reads).toBeGreaterThan(0);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(12500);
    });
    const visibleReads = reads;
    expect(visibleReads).toBeGreaterThanOrEqual(3);
    const subscriptionState = store.getState().buddyApi as unknown as {
      subscriptions: Record<
        string,
        Record<string, { pollingInterval?: number }>
      >;
    };
    expect(
      Object.values(subscriptionState.subscriptions)
        .flatMap((entries) => Object.values(entries))
        .some((s) => s.pollingInterval === 5000),
    ).toBe(true);

    act(() => {
      visibilityState = "hidden";
      document.dispatchEvent(new Event("visibilitychange"));
      window.dispatchEvent(new Event("visibilitychange"));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(15000);
    });
    expect(reads).toBe(visibleReads);

    act(() => {
      visibilityState = "visible";
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
    stopListeners();
    view.unmount();
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
