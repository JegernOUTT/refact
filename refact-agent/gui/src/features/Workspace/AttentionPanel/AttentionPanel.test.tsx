import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, fireEvent } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { AttentionPanel } from "./AttentionPanel";
import type {
  AttentionEntry,
  AttentionQueue,
} from "../../../services/refact/attentionTypes";

afterEach(() => vi.restoreAllMocks());

const renderPanel = () =>
  render(<AttentionPanel />, {
    preloadedState: {
      config: {
        host: "ide",
        lspPort: 8001,
        themeProps: { appearance: "dark" },
      },
    },
  });

function queueHandler(queue: Record<string, unknown> | AttentionQueue) {
  server.use(http.get("*/v1/attention", () => HttpResponse.json(queue)));
}

const entry = (overrides: Partial<AttentionEntry> = {}): AttentionEntry => ({
  question_id: "q1",
  text: "Ship it?",
  kind: "user_input",
  priority: "normal",
  state: "waiting",
  rank: 1,
  rank_reason: "1 run blocked · normal priority",
  blocked_runs: 1,
  waiters: ["agent-one"],
  asked_by: "agent-one",
  merged_waiters: [],
  presented: false,
  irreversible: false,
  default_answer: null,
  default_used: null,
  settled_at_ms: null,
  project: "/project",
  asked_at_ms: 1_000,
  ...overrides,
});

const queue = (overrides: Partial<AttentionQueue> = {}): AttentionQueue => ({
  waiting: [],
  settled: [],
  outstanding: 0,
  cap: 8,
  batch_window_ms: 1_000,
  default_timeout_secs: 60,
  silent_defaults: 0,
  generated_at_ms: 2_000,
  ...overrides,
});

describe("Attention panel", () => {
  it("renders an explicit unavailable state rather than an empty success", async () => {
    server.use(
      http.get("*/v1/attention", () =>
        HttpResponse.json({ code: "unauthorized" }, { status: 401 }),
      ),
    );
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Attention queue unavailable",
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Authentication is required",
    );
    queueHandler(queue());
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(
      await screen.findByText("Nothing is waiting on you"),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Waiting questions")).toBeNull();
  });

  it("says nothing is waiting when nothing is waiting", async () => {
    queueHandler(queue());
    renderPanel();
    expect(
      await screen.findByText("Nothing is waiting on you"),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Silent defaults")).toBeNull();
  });

  it("ranks one question first and names the reason for the rank", async () => {
    queueHandler(queue({ waiting: [entry()], outstanding: 1 }));
    renderPanel();
    expect(await screen.findByText("Rank 1")).toBeInTheDocument();
    expect(screen.queryByText(/Defaults to/)).not.toBeInTheDocument();
    expect(screen.getByLabelText("Rank reason 1")).toHaveTextContent(
      "1 run blocked · normal priority",
    );
  });

  it("ranks many questions by blocked work and shows the same order for the same input", async () => {
    const page = queue({
      outstanding: 3,
      waiting: [
        entry({
          question_id: "crowded",
          text: "Crowded?",
          rank: 1,
          blocked_runs: 3,
          rank_reason: "3 runs blocked · normal priority",
          waiters: ["agent-one", "agent-two", "agent-three"],
          merged_waiters: ["agent-two", "agent-three"],
        }),
        entry({
          question_id: "urgent",
          text: "Urgent?",
          rank: 2,
          priority: "blocking",
          rank_reason:
            "1 run blocked · blocking priority skips the batching window",
        }),
        entry({ question_id: "lonely", text: "Lonely?", rank: 3 }),
      ],
    });
    queueHandler(page);
    const { unmount } = renderPanel();
    const order = async () =>
      (await screen.findAllByText(/^Rank \d+$/)).map(
        (node) => node.textContent,
      );
    expect(await order()).toEqual(["Rank 1", "Rank 2", "Rank 3"]);
    const deduped = screen.getByLabelText("Deduped into crowded");
    expect(deduped).toHaveTextContent("Asked by agent-one");
    expect(deduped).toHaveTextContent(
      "agent-two was merged into this same question",
    );
    unmount();

    queueHandler(page);
    renderPanel();
    expect(await order()).toEqual(["Rank 1", "Rank 2", "Rank 3"]);
  });

  it("marks a question that timed out into its default as exactly that", async () => {
    queueHandler(
      queue({
        silent_defaults: 1,
        settled: [
          entry({
            question_id: "renamed",
            text: "Rename the branch?",
            state: "defaulted",
            priority: "low",
            rank: 0,
            default_answer: "keep the name",
            default_used: "keep the name",
            settled_at_ms: 3_000,
          }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByLabelText("Silent defaults")).toHaveTextContent(
      "1 question timed out into a default instead of being answered",
    );
    expect(
      screen.getByText(/was applied without your consent/),
    ).toHaveTextContent(
      'The documented default "keep the name" was applied without your consent',
    );
    expect(screen.getByText("Answered by timeout")).toBeInTheDocument();
  });

  it("never shows an answered question as a silent default", async () => {
    queueHandler(
      queue({
        settled: [
          entry({
            question_id: "answered",
            text: "Ship it?",
            state: "answered",
            rank: 0,
            settled_at_ms: 3_000,
          }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByText("Ship it?")).toBeInTheDocument();
    expect(screen.queryByLabelText("Silent defaults")).toBeNull();
    expect(screen.queryByText(/without your consent/)).toBeNull();
  });

  it("refuses a snapshot that reports a default without naming what was applied", async () => {
    queueHandler(
      queue({
        silent_defaults: 1,
        settled: [
          entry({
            question_id: "renamed",
            state: "defaulted",
            rank: 0,
            settled_at_ms: 3_000,
          }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Attention queue unavailable",
    );
  });

  it("refuses a snapshot that merges the asker into its own question", async () => {
    queueHandler(
      queue({
        outstanding: 1,
        waiting: [
          entry({ asked_by: "agent-one", merged_waiters: ["agent-one"] }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Attention queue unavailable",
    );
  });

  it("still refuses null required fields rather than treating them as an empty queue", async () => {
    queueHandler({
      ...queue({ waiting: [entry()], outstanding: 1 }),
      cap: null,
    });
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "unreadable snapshot",
    );
    expect(screen.queryByText("Ship it?")).not.toBeInTheDocument();
  });

  it("says the human is saturated when the cap is reached", async () => {
    queueHandler(queue({ waiting: [entry()], outstanding: 8, cap: 8 }));
    renderPanel();
    expect(
      await screen.findByText(/new low-priority questions are refused/),
    ).toBeInTheDocument();
  });
});
