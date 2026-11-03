import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { act, fireEvent } from "@testing-library/react";
import {
  render,
  screen,
  waitFor,
  createDefaultChatState,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import type {
  AttentionRow,
  AttentionSnapshot,
} from "../../../services/refact/buddyCompanion";
import { BuddyCompanion, buddyNamespace } from "./BuddyCompanion";
import { BuddyRailButton } from "./BuddyRailButton";
import {
  resetBuddyCompanion,
  updateBuddyCompanion,
} from "./buddyCompanionStore";
import { popoverStorageKey } from "./popoverGeometry";

const navigate = vi.fn((url: string) => Promise.resolve(url));
vi.mock("../../../hooks/useInternalNavigation", () => ({
  useInternalNavigation: () => ({
    navigate,
    navigateKnownAgent: vi.fn(),
    navigateLocalTaskChat: vi.fn(),
    pending: false,
    showProgress: false,
    error: null,
  }),
}));

const config = {
  host: "vscode" as const,
  lspPort: 8001,
  themeProps: {},
};

function row(index: number, patch: Partial<AttentionRow> = {}): AttentionRow {
  return {
    id: `row-${index}`,
    kind: "tool_permission",
    chat_id: `chat-${index}`,
    source_version: "v1",
    reason: `Reason ${index} needs a decision before the agent can continue`,
    effect: "execute_pending_tool",
    actionable: true,
    literal_command: null,
    display_command: null,
    cwd: null,
    tool_call_id: `tool-${index}`,
    question: null,
    parent_chat_id: null,
    run_generation: null,
    expires_at: null,
    ...patch,
  };
}

function snapshot(rows: AttentionRow[]): AttentionSnapshot {
  return {
    schema_version: 1,
    project_root: "/fixture",
    engine_identity: "fixture",
    coverage: "full",
    partial: false,
    busy_chat_ids: [],
    rows,
  };
}

function attentionHandler(rows: AttentionRow[]) {
  return http.get("*/v1/buddy/attention", () =>
    HttpResponse.json(snapshot(rows)),
  );
}

function renderBuddy(rows: AttentionRow[] = []) {
  server.use(attentionHandler(rows));
  return render(
    <>
      <nav>
        <BuddyRailButton />
      </nav>
      <label>
        Main draft
        <textarea />
      </label>
      <BuddyCompanion />
    </>,
    {
      preloadedState: {
        config,
        chat: createDefaultChatState(),
        pages: [{ name: "history" }],
      },
    },
  );
}

beforeEach(() => {
  resetBuddyCompanion();
  localStorage.clear();
  navigate.mockClear();
  Object.defineProperty(window, "innerWidth", {
    configurable: true,
    value: 1100,
  });
  Object.defineProperty(window, "innerHeight", {
    configurable: true,
    value: 860,
  });
});

afterEach(() => {
  resetBuddyCompanion();
});

describe("Buddy rail button", () => {
  it("renders the live avatar with an attention badge and accessible name", async () => {
    renderBuddy([row(1), row(2)]);
    const button = await screen.findByRole("button", {
      name: "Buddy, 2 need you",
    });
    expect(button.querySelector("canvas")).not.toBeNull();
    expect(button).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByTestId("buddy-attention-badge")).toHaveTextContent("2");
  });

  it("shows a plain label without a badge when nothing needs attention", async () => {
    renderBuddy();
    expect(
      await screen.findByRole("button", { name: "Buddy" }),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("buddy-attention-badge")).toBeNull();
  });

  it("shows an unread reply dot next to the attention badge", async () => {
    renderBuddy([row(1)]);
    await screen.findByTestId("buddy-attention-badge");
    act(() => updateBuddyCompanion({ unread: true }));
    expect(
      screen.getByRole("button", { name: "Buddy, 1 needs you, new reply" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("buddy-unread-dot")).toBeInTheDocument();
    expect(screen.getByTestId("buddy-attention-badge")).toHaveTextContent("1");
  });

  it("toggles the popover", async () => {
    const { user } = renderBuddy();
    const button = screen.getByRole("button", { name: "Buddy" });
    await user.click(button);
    expect(button).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("dialog", { name: "Buddy" })).toHaveAttribute(
      "aria-modal",
      "false",
    );
    await user.click(button);
    expect(screen.queryByRole("dialog", { name: "Buddy" })).toBeNull();
  });
});

describe("Buddy popover", () => {
  it("is non-modal: a textbox outside stays focusable and typable", async () => {
    const { user } = renderBuddy();
    await user.click(screen.getByRole("button", { name: "Buddy" }));
    const main = screen.getByRole("textbox", { name: "Main draft" });
    await user.click(main);
    await user.type(main, "still typing");
    expect(main).toHaveFocus();
    expect(main).toHaveValue("still typing");
    expect(screen.getByRole("dialog", { name: "Buddy" })).toBeInTheDocument();
  });

  it("shows the no-source hint without jargon", async () => {
    const { user } = renderBuddy();
    await user.click(screen.getByRole("button", { name: "Buddy" }));
    expect(
      screen.getByText("Open a chat to talk to Buddy about it."),
    ).toBeInTheDocument();
    const dialog = screen.getByRole("dialog", { name: "Buddy" });
    expect(dialog).not.toHaveTextContent(/coverage|partial|busy chats/i);
  });

  it("minimizes on Escape and returns focus to the rail icon", async () => {
    const { user } = renderBuddy();
    const button = screen.getByRole("button", { name: "Buddy" });
    await user.click(button);
    const dialog = screen.getByRole("dialog", { name: "Buddy" });
    expect(dialog).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Buddy" })).toBeNull();
    expect(button).toHaveFocus();
  });

  it("minimizes on Escape after focus falls back to the page", async () => {
    const { user } = renderBuddy();
    const button = screen.getByRole("button", { name: "Buddy" });
    await user.click(button);
    act(() => {
      if (document.activeElement instanceof HTMLElement)
        document.activeElement.blur();
    });
    expect(document.activeElement).toBe(document.body);
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "Buddy" })).toBeNull();
    expect(button).toHaveFocus();
  });

  it("minimize button closes and returns focus to the rail icon", async () => {
    const { user } = renderBuddy();
    const button = screen.getByRole("button", { name: "Buddy" });
    await user.click(button);
    await user.click(screen.getByRole("button", { name: "Minimize Buddy" }));
    expect(screen.queryByRole("dialog", { name: "Buddy" })).toBeNull();
    expect(button).toHaveFocus();
  });

  it("drags by the header, clamps to the viewport and persists geometry", async () => {
    const { user } = renderBuddy();
    const key = popoverStorageKey(buddyNamespace(config));
    localStorage.setItem(
      key,
      JSON.stringify({ x: 100, y: 120, width: 420, height: 500 }),
    );
    await user.click(screen.getByRole("button", { name: "Buddy" }));
    const dialog = screen.getByRole("dialog", { name: "Buddy" });
    const header = screen.getByTestId("buddy-popover-header");
    const startLeft = parseFloat(dialog.style.left);
    const startTop = parseFloat(dialog.style.top);
    fireEvent.pointerDown(header, {
      pointerId: 1,
      button: 0,
      clientX: 100,
      clientY: 100,
    });
    fireEvent.pointerMove(header, { pointerId: 1, clientX: 50, clientY: 80 });
    expect(parseFloat(dialog.style.left)).toBe(startLeft - 50);
    expect(parseFloat(dialog.style.top)).toBe(startTop - 20);
    fireEvent.pointerMove(header, {
      pointerId: 1,
      clientX: 5000,
      clientY: 5000,
    });
    const width = parseFloat(dialog.style.width);
    const height = parseFloat(dialog.style.height);
    expect(parseFloat(dialog.style.left)).toBe(1100 - width - 8);
    expect(parseFloat(dialog.style.top)).toBe(860 - height - 8);
    fireEvent.pointerUp(header, { pointerId: 1 });
    expect(JSON.parse(localStorage.getItem(key) ?? "null")).toEqual({
      x: 1100 - width - 8,
      y: 860 - height - 8,
      width,
      height,
    });
  });

  it("moves and resizes with arrow keys and saves the result", async () => {
    const { user } = renderBuddy();
    const key = popoverStorageKey(buddyNamespace(config));
    localStorage.setItem(
      key,
      JSON.stringify({ x: 100, y: 120, width: 420, height: 500 }),
    );
    await user.click(screen.getByRole("button", { name: "Buddy" }));
    const dialog = screen.getByRole("dialog", { name: "Buddy" });
    const move = screen.getByRole("button", { name: /^Move Buddy/ });
    fireEvent.keyDown(move, { key: "ArrowRight" });
    fireEvent.keyDown(move, { key: "ArrowDown", shiftKey: true });
    fireEvent.keyDown(screen.getByRole("button", { name: "Resize Buddy" }), {
      key: "ArrowLeft",
    });
    expect(JSON.parse(localStorage.getItem(key) ?? "null")).toEqual({
      x: 116,
      y: 184,
      width: 404,
      height: 500,
    });
    expect(parseFloat(dialog.style.left)).toBe(116);
    expect(parseFloat(dialog.style.width)).toBe(404);
  });

  it("restores the chosen geometry after a temporary window shrink", async () => {
    const { user } = renderBuddy();
    const key = popoverStorageKey(buddyNamespace(config));
    const saved = { x: 600, y: 120, width: 420, height: 500 };
    localStorage.setItem(key, JSON.stringify(saved));
    await user.click(screen.getByRole("button", { name: "Buddy" }));
    const dialog = screen.getByRole("dialog", { name: "Buddy" });
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 800,
    });
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(parseFloat(dialog.style.left)).toBe(800 - 420 - 8);
    expect(JSON.parse(localStorage.getItem(key) ?? "null")).toEqual(saved);
    Object.defineProperty(window, "innerWidth", {
      configurable: true,
      value: 1100,
    });
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(parseFloat(dialog.style.left)).toBe(600);
  });

  it("ignores drags that start on header buttons", async () => {
    const { user } = renderBuddy();
    await user.click(screen.getByRole("button", { name: "Buddy" }));
    const dialog = screen.getByRole("dialog", { name: "Buddy" });
    const left = dialog.style.left;
    const minimize = screen.getByRole("button", { name: "Minimize Buddy" });
    fireEvent.pointerDown(minimize, {
      pointerId: 2,
      button: 0,
      clientX: 10,
      clientY: 10,
    });
    fireEvent.pointerMove(minimize, {
      pointerId: 2,
      clientX: 300,
      clientY: 10,
    });
    expect(dialog.style.left).toBe(left);
  });

  it("resizes from the corner with a minimum size", async () => {
    const { user } = renderBuddy();
    await user.click(screen.getByRole("button", { name: "Buddy" }));
    const dialog = screen.getByRole("dialog", { name: "Buddy" });
    const handle = screen.getByTestId("buddy-resize-handle");
    fireEvent.pointerDown(handle, {
      pointerId: 3,
      button: 0,
      clientX: 500,
      clientY: 500,
    });
    fireEvent.pointerMove(handle, { pointerId: 3, clientX: 0, clientY: 0 });
    expect(parseFloat(dialog.style.width)).toBe(320);
    expect(parseFloat(dialog.style.height)).toBe(360);
  });

  it("lists at most three rows, expands with Show all and navigates on click", async () => {
    const rows = [
      row(1, { parent_chat_id: "parent-1" }),
      row(2),
      row(3),
      row(4),
      row(5),
    ];
    const { user } = renderBuddy(rows);
    await screen.findByRole("button", { name: "Buddy, 5 need you" });
    await user.click(screen.getByRole("button", { name: "Buddy, 5 need you" }));
    const list = screen.getByRole("region", { name: "Needs you" });
    expect(list.querySelectorAll("li")).toHaveLength(3);
    const first = screen.getByRole("button", { name: /Reason 1/ });
    expect(first).toHaveAttribute("title", expect.stringContaining("Reason 1"));
    expect(first).toHaveTextContent("Tool permission");
    await user.click(screen.getByRole("button", { name: "Show all (5)" }));
    expect(list.querySelectorAll("li")).toHaveLength(5);
    await user.click(first);
    expect(navigate).toHaveBeenCalledWith("refact://chat/parent-1");
    await user.click(screen.getByRole("button", { name: /Reason 2/ }));
    expect(navigate).toHaveBeenLastCalledWith("refact://chat/chat-2");
    expect(screen.getByRole("dialog", { name: "Buddy" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /approve/i })).toBeNull();
  });
});

describe("Buddy attention polling", () => {
  it("stops polling after a 404", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const requests = vi.fn();
      server.use(
        http.get("*/v1/buddy/attention", () => {
          requests();
          return new HttpResponse("missing", { status: 404 });
        }),
      );
      render(<BuddyCompanion />, {
        preloadedState: {
          config,
          chat: createDefaultChatState(),
          pages: [{ name: "history" }],
        },
      });
      await waitFor(() => expect(requests).toHaveBeenCalledTimes(1));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(70_000);
      });
      expect(requests).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps polling healthy engines", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const requests = vi.fn();
      server.use(
        http.get("*/v1/buddy/attention", () => {
          requests();
          return HttpResponse.json(snapshot([]));
        }),
      );
      render(<BuddyCompanion />, {
        preloadedState: {
          config,
          chat: createDefaultChatState(),
          pages: [{ name: "history" }],
        },
      });
      await waitFor(() => expect(requests).toHaveBeenCalledTimes(1));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_100);
      });
      await waitFor(() => expect(requests).toHaveBeenCalledTimes(2));
    } finally {
      vi.useRealTimers();
    }
  });
});
