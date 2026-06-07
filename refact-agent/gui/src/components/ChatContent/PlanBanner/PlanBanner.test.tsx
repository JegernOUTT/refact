import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "../../../utils/test-utils";
import type { RootState } from "../../../app/store";
import type {
  Chat,
  ChatThreadRuntime,
} from "../../../features/Chat/Thread/types";
import type {
  ChatMessages,
  EventMessage,
  PlanMessage,
} from "../../../services/refact/types";
import { PlanBanner } from "./PlanBanner";

const threadId = "plan-banner-thread";
const nowMs = 1_700_000_000_000;
let writeTextMock: ReturnType<typeof vi.fn<(text: string) => Promise<void>>>;

function makePlan(
  version: number,
  overrides: Partial<PlanMessage> = {},
): PlanMessage {
  return {
    role: "plan",
    message_id: `plan-${version}`,
    content: `## Plan ${version}\n\n- item ${version}`,
    extra: {
      plan: {
        mode: "agent",
        version,
        created_at_ms: nowMs - 2 * 60_000,
      },
    },
    ...overrides,
  };
}

function makePlanDelta(content: string, messageId: string): EventMessage {
  return {
    role: "event",
    message_id: messageId,
    content,
    subkind: "plan_delta",
    source: "test",
  };
}

function makeRuntime(messages: ChatMessages): ChatThreadRuntime {
  return {
    thread: {
      id: threadId,
      messages,
      title: "Plan Banner Chat",
      model: "gpt-4",
      tool_use: "agent",
      new_chat_suggested: { wasSuggested: false },
      boost_reasoning: false,
      increase_max_tokens: false,
      include_project_info: true,
    },
    streaming: false,
    waiting_for_response: false,
    prevent_send: false,
    error: null,
    queued_items: [],
    send_immediately: false,
    attached_images: [],
    attached_text_files: [],
    background_agents: {},
    confirmation: {
      pause: false,
      pause_reasons: [],
      status: {
        wasInteracted: false,
        confirmationStatus: true,
      },
    },
    snapshot_received: true,
    task_widget_expanded: false,
    memory_enrichment_user_touched: false,
    manual_preview_items: [],
    manual_preview_ran: false,
  };
}

function makeChatState(messages: ChatMessages): Chat {
  return {
    current_thread_id: threadId,
    open_thread_ids: [threadId],
    threads: { [threadId]: makeRuntime(messages) },
    system_prompt: {},
    tool_use: "agent",
    sse_refresh_requested: null,
    stream_version: 0,
  };
}

function renderPlanBanner(messages: ChatMessages) {
  return render(<PlanBanner threadId={threadId} />, {
    preloadedState: { chat: makeChatState(messages) } as Partial<RootState>,
  });
}

describe("PlanBanner", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.restoreAllMocks();
    writeTextMock = vi.fn<(text: string) => Promise<void>>(() =>
      Promise.resolve(),
    );
    Object.defineProperty(window.navigator.clipboard, "writeText", {
      configurable: true,
      value: writeTextMock,
    });
    vi.spyOn(Date, "now").mockReturnValue(nowMs);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders nothing when no plan in state", () => {
    renderPlanBanner([
      { role: "assistant", content: "hello", message_id: "assistant-1" },
    ]);

    expect(screen.queryByTestId("plan-banner")).toBeNull();
  });

  it("renders header with mode, version, and humanized age for one plan", () => {
    renderPlanBanner([makePlan(1)]);

    expect(screen.getByText("Plan — agent · v1 · 2m ago")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Plan 1" })).toBeTruthy();
    expect(screen.getByText("item 1")).toBeTruthy();
    expect(screen.queryByText("Edit plan")).toBeNull();
  });

  it("copies synthesized plan text without toggling collapse", () => {
    renderPlanBanner([makePlan(1), makePlanDelta("first update", "delta-1")]);

    fireEvent.click(screen.getByRole("button", { name: "Copy plan" }));

    expect(writeTextMock).toHaveBeenCalledWith(
      "## Plan 1\n\n- item 1\n\n---\n\n## Plan updates\n\nfirst update",
    );
    expect(screen.getByTestId("plan-banner-body")).toBeTruthy();
  });

  it("renders synthesized plan text with ordered updates", () => {
    renderPlanBanner([
      makePlan(1),
      makePlanDelta("first update", "delta-1"),
      makePlanDelta("second update", "delta-2"),
    ]);

    expect(screen.getByRole("heading", { name: "Plan 1" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Plan updates" })).toBeTruthy();
    expect(screen.getByText("first update")).toBeTruthy();
    expect(screen.getByText("second update")).toBeTruthy();
  });

  it("opens history with base plan and delta notes", () => {
    renderPlanBanner([
      makePlan(1),
      makePlanDelta("first update", "delta-1"),
      makePlanDelta("second update", "delta-2"),
    ]);

    fireEvent.click(screen.getByRole("button", { name: "History" }));

    expect(screen.getByRole("heading", { name: "Plan history" })).toBeTruthy();
    expect(screen.getByText("📋 Base plan — agent · v1")).toBeTruthy();
    expect(screen.getByText("📋 Plan update 1")).toBeTruthy();
    expect(screen.getByText("📋 Plan update 2")).toBeTruthy();
    expect(screen.queryByText("Edit plan")).toBeNull();
  });

  it("renders graceful fallbacks when plan metadata fields are missing", () => {
    renderPlanBanner([
      makePlan(1, {
        extra: { plan: {} },
      }),
    ]);

    const header = screen.getByText("Plan — Mode unknown · v? · recently");
    expect(header.textContent).not.toContain("undefined");
    expect(header.textContent).not.toContain("vundefined");
    expect(header.textContent).not.toContain("NaN");
  });

  it("renders task_done plan payloads as markdown report text", () => {
    renderPlanBanner([
      makePlan(1, {
        content: JSON.stringify({
          type: "task_done",
          summary: "Completed the task",
          report: "## What changed\n\nEverything is done.",
          files_changed: ["src/foo.ts"],
        }),
      }),
    ]);

    expect(screen.getByText("Completed the task")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "What changed" })).toBeTruthy();
    expect(screen.getByText("Everything is done.")).toBeTruthy();
    expect(screen.getByText("src/foo.ts")).toBeTruthy();
    expect(screen.queryByText(/task_done/)).toBeNull();
  });

  it("toggle collapse hides body, persists in localStorage, and restores on remount", () => {
    const { unmount } = renderPlanBanner([makePlan(1)]);

    fireEvent.click(screen.getByTestId("plan-banner-header"));

    expect(screen.queryByTestId("plan-banner-body")).toBeNull();
    expect(localStorage.getItem(`plan-banner-collapsed-${threadId}`)).toBe(
      "true",
    );

    unmount();
    renderPlanBanner([makePlan(1)]);

    expect(screen.queryByTestId("plan-banner-body")).toBeNull();
    expect(screen.getByTestId("plan-banner-header")).toBeTruthy();
  });

  it("compact plan classes are applied", () => {
    renderPlanBanner([makePlan(1)]);

    const banner = screen.getByTestId("plan-banner");
    const card = screen.getByTestId("plan-banner-card");
    const body = screen.getByTestId("plan-banner-body");
    expect(banner.className).toContain("sticky");
    expect(banner.firstElementChild?.className).toContain("container");
    expect(card.className).toContain("card");
    expect(body.className).toContain("body");
    expect(body).toBeTruthy();
  });
});
