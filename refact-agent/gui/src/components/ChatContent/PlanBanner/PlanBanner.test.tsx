import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "../../../utils/test-utils";
import type { RootState } from "../../../app/store";
import type {
  Chat,
  ChatThreadRuntime,
} from "../../../features/Chat/Thread/types";
import type { ChatMessages, PlanMessage } from "../../../services/refact/types";
import { PlanBanner } from "./PlanBanner";

const threadId = "plan-banner-thread";
const nowMs = 1_700_000_000_000;

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

    expect(screen.getByText("📋 Plan — agent · v1 · 2m ago")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Plan 1" })).toBeTruthy();
    expect(screen.getByText("item 1")).toBeTruthy();
  });

  it("renders graceful fallbacks when plan metadata fields are missing", () => {
    renderPlanBanner([
      makePlan(1, {
        extra: { plan: {} },
      }),
    ]);

    const header = screen.getByText("📋 Plan — Mode unknown · v? · recently");
    expect(header.textContent).not.toContain("undefined");
    expect(header.textContent).not.toContain("vundefined");
    expect(header.textContent).not.toContain("NaN");
  });

  it("toggle collapse hides body, persists in localStorage, and restores on remount", () => {
    const { unmount } = renderPlanBanner([makePlan(1)]);

    fireEvent.click(screen.getByRole("button", { name: "Collapse" }));

    expect(screen.queryByTestId("plan-banner-body")).toBeNull();
    expect(localStorage.getItem(`plan-banner-collapsed-${threadId}`)).toBe(
      "true",
    );

    unmount();
    renderPlanBanner([makePlan(1)]);

    expect(screen.queryByTestId("plan-banner-body")).toBeNull();
    expect(screen.getByRole("button", { name: "Expand" })).toBeTruthy();
  });

  it("History button opens modal listing all plan versions", () => {
    renderPlanBanner([
      makePlan(1),
      makePlan(3, { content: "plan three" }),
      makePlan(2, { content: "plan two" }),
    ]);

    fireEvent.click(screen.getByRole("button", { name: "History" }));

    const dialog = screen.getByRole("dialog", { name: "Plan history" });
    expect(within(dialog).getByText("📋 Plan — agent · v3")).toBeTruthy();
    expect(within(dialog).getByText("📋 Plan — agent · v2")).toBeTruthy();
    expect(within(dialog).getByText("📋 Plan — agent · v1")).toBeTruthy();

    const headers = within(dialog).getAllByText(/📋 Plan — agent · v/u);
    expect(headers.map((header) => header.textContent)).toEqual([
      "📋 Plan — agent · v3",
      "📋 Plan — agent · v2",
      "📋 Plan — agent · v1",
    ]);
  });

  it("modal close button works", () => {
    renderPlanBanner([makePlan(1)]);

    fireEvent.click(screen.getByRole("button", { name: "History" }));
    expect(screen.getByRole("dialog", { name: "Plan history" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));

    expect(screen.queryByRole("dialog", { name: "Plan history" })).toBeNull();
  });

  it("sticky CSS class is applied to the banner root", () => {
    renderPlanBanner([makePlan(1)]);

    const banner = screen.getByTestId("plan-banner");
    expect(banner.className).toContain("sticky");
  });
});
