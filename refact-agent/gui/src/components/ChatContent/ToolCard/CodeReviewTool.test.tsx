import { describe, expect, test, vi } from "vitest";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
} from "../../../utils/test-utils";
import type { ChatMessage, ToolCall } from "../../../services/refact/types";
import { CodeReviewTool } from "./CodeReviewTool";
import { CollapsibleStoreProvider } from "../useStoredOpen";

vi.hoisted(() => vi.resetModules());
const openFile = vi.hoisted(() => vi.fn());
vi.mock("../../../hooks/useOpenFileInApp", () => ({
  useOpenFileInApp: () => ({ canOpen: true, openFile }),
}));

function toolCall(id = "review-card-1"): ToolCall {
  return {
    id,
    index: 0,
    function: { name: "review", arguments: "{}" },
  };
}

function reviewReport() {
  return {
    depth: "deep",
    scope: {
      mode: "strict",
      requested_files: 2,
      reviewed_files: 3,
      files: ["src/cache.ts"],
      focus: "Concurrency correctness",
      expansion: null,
      out_of_scope_findings: 0,
    },
    diff: { base: "abc123", head: "HEAD", changed_files: 2, hunks: 7 },
    stages: [
      {
        name: "diff",
        model: "anthropic/claude-sonnet-review",
        status: "ok",
        duration_ms: 4000,
        findings: 1,
        summary: "read two files",
        coverage: {
          files_read: ["src/cache.ts"],
          commands_run: [],
          tools_unavailable: [],
          stopped_early: null,
        },
      },
      {
        name: "tests",
        model: null,
        status: "timed_out",
        reason: "stage budget of 360s exceeded",
        duration_ms: 360000,
        findings: 0,
        summary: null,
        coverage: {
          files_read: [],
          commands_run: [],
          tools_unavailable: [],
          stopped_early: null,
        },
      },
    ],
    findings: [
      {
        id: "rf-1",
        stage: "diff",
        model: "anthropic/claude-sonnet-review",
        title: "Race",
        severity: "high",
        file: "src/cache.ts",
        line_start: 42,
        line_end: 46,
        claim: "Concurrent writes can overwrite a newer cache entry.",
        evidence: "writerA();\nwriterB();",
        evidence_present: true,
        reproduction: "npm test -- cache",
        fix: "Serialize updates by key.",
        introduced_by_diff: true,
        out_of_scope: false,
        reported_by: ["diff"],
        locations: [],
        disputed: null,
      },
      {
        id: "rf-2",
        stage: "spec",
        model: null,
        title: "Maybe stale docs",
        severity: "note",
        file: "docs/cache.md",
        line_start: 3,
        line_end: 3,
        claim: "The documentation may be stale.",
        evidence: "",
        evidence_present: false,
        reproduction: null,
        fix: null,
        introduced_by_diff: false,
        out_of_scope: false,
        reported_by: ["spec"],
        locations: [],
        disputed: null,
      },
    ],
    duration_ms: 252000,
    duplicates_merged: 1,
    scratch_dir: ".refact/review_scratch/rv-1",
  };
}

function inconclusiveReport() {
  return {
    depth: "deep",
    outcome: "inconclusive",
    scope: {
      mode: "strict",
      requested_files: 2,
      reviewed_files: 0,
      files: [],
      dropped_files: ["src/cache.ts", "src/other.ts"],
      focus: null,
      expansion: null,
      out_of_scope_findings: 0,
    },
    diff: { base: "abc123", head: "HEAD", changed_files: 2, hunks: 7 },
    stages: [
      {
        name: "diff",
        model: null,
        status: "timed_out",
        reason: "stage budget of 360s exceeded",
        duration_ms: 360000,
        findings: 0,
        summary: null,
        trace_chat_id: "trace-diff-1",
        coverage: {
          files_read: [],
          commands_run: [],
          tools_unavailable: [],
          stopped_early: null,
        },
      },
      {
        name: "tests",
        model: null,
        status: "timed_out",
        reason: "stage budget of 360s exceeded",
        duration_ms: 360000,
        findings: 0,
        summary: null,
        trace_chat_id: "trace-tests-2",
        coverage: {
          files_read: [],
          commands_run: [],
          tools_unavailable: [],
          stopped_early: null,
        },
      },
    ],
    findings: [],
    duration_ms: 720000,
    duplicates_merged: 0,
    scratch_dir: null,
  };
}

function message(
  content: string,
  extra?: Record<string, unknown>,
  failed = false,
): ChatMessage {
  return {
    role: "tool",
    tool_call_id: "review-card-1",
    content,
    tool_failed: failed,
    ...(extra ? { extra } : {}),
  } as ChatMessage;
}

function renderReview(messageValue?: ChatMessage, call = toolCall()) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.messages = messageValue ? [messageValue] : [];
  return render(<CodeReviewTool toolCall={call} />, {
    preloadedState: { chat },
  });
}

function openReview() {
  fireEvent.click(screen.getByRole("button", { expanded: false }));
  fireEvent.click(screen.getByRole("button", { name: "More details" }));
}

describe("CodeReviewTool", () => {
  test("renders the structured report from tool metering", () => {
    renderReview(
      message("## Review · 2 file(s) requested", {
        review_report: reviewReport(),
      }),
    );

    expect(screen.queryByTestId("review-report")).not.toBeInTheDocument();
    expect(screen.getByRole("button")).toHaveTextContent(
      "3 files reviewed · 2 findings",
    );
    expect(screen.getByRole("button")).toHaveTextContent("Partial review");
    openReview();

    expect(screen.getByTestId("review-report")).toBeInTheDocument();
    expect(
      screen.getByText(
        "1 supported (1 reproduced) · 1 hypotheses · 1 duplicates merged · 0 out of scope",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Concurrent writes can overwrite a newer cache entry."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Hypotheses — unverified, you decide"),
    ).toBeInTheDocument();
    expect(screen.getAllByText("timed out").length).toBeGreaterThan(0);
    expect(
      screen.getByText(
        "Partial review: 1 stage(s) did not complete — absence of findings there is not evidence of absence.",
      ),
    ).toBeInTheDocument();
  });

  test("legacy report without outcome renders unchanged, with no banner", () => {
    renderReview(
      message("## Review · 2 file(s) requested", {
        review_report: reviewReport(),
      }),
    );
    openReview();

    expect(
      screen.queryByTestId("review-outcome-banner"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("review-dropped-files"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(
        "Partial review: 1 stage(s) did not complete — absence of findings there is not evidence of absence.",
      ),
    ).toBeInTheDocument();
  });

  test("an inconclusive report shows an error banner and no clean summary", () => {
    renderReview(
      message("## Review · 2 file(s) requested", {
        review_report: inconclusiveReport(),
      }),
    );
    expect(screen.getByText("Review unavailable")).toBeInTheDocument();
    openReview();

    const banner = screen.getByTestId("review-outcome-banner");
    expect(banner).toHaveAttribute("role", "alert");
    expect(banner).toHaveTextContent(
      "Review inconclusive — no stage completed and nothing was verified. This is not a pass.",
    );
    expect(banner).toHaveTextContent(
      "diff — timed out: stage budget of 360s exceeded",
    );
    expect(banner).toHaveTextContent("trace: trace-diff-1");
    expect(banner).toHaveTextContent("trace: trace-tests-2");
    expect(
      screen.queryByText("No supported findings."),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText(
        /absence of findings there is not evidence of absence/,
      ),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("review-dropped-files")).toHaveTextContent(
      "2 files were not reviewed",
    );
    expect(screen.getByText("src/cache.ts")).toBeInTheDocument();
    expect(screen.getByText("src/other.ts")).toBeInTheDocument();
  });

  test("a partial report shows a warning banner with completed stage counts", () => {
    renderReview(
      message("## Review · 2 file(s) requested", {
        review_report: { ...reviewReport(), outcome: "partial" },
      }),
    );
    openReview();

    const banner = screen.getByTestId("review-outcome-banner");
    expect(banner).toHaveAttribute("role", "status");
    expect(banner).toHaveTextContent(
      "Partial review: 1 of 2 stages completed — findings may be incomplete.",
    );
    expect(banner).toHaveTextContent(
      "tests — timed out: stage budget of 360s exceeded",
    );
    expect(
      screen.queryByText(
        "Partial review: 1 stage(s) did not complete — absence of findings there is not evidence of absence.",
      ),
    ).not.toBeInTheDocument();
  });

  test("falls back to markdown without a report", () => {
    renderReview(message("# Review notes\n\nNo blocking issues found."));

    expect(screen.getByText("Review unavailable")).toBeInTheDocument();
    expect(
      screen.queryByText("No blocking issues found."),
    ).not.toBeInTheDocument();
    openReview();
    expect(screen.getByText("Review notes")).toBeInTheDocument();
    expect(screen.getByText("No blocking issues found.")).toBeInTheDocument();
  });

  test("shows error status when the tool failed", () => {
    const { container } = renderReview(
      message(
        "# Review failed\n\nUnable to inspect the diff.",
        undefined,
        true,
      ),
    );

    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "error",
    );
    expect(screen.getByText("Request failed")).toBeInTheDocument();
    expect(screen.getByRole("button")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  test("shows three facts first and keeps evidence, models, depth and file navigation deeper", async () => {
    const { container, user } = renderReview(
      message("original report", { review_report: reviewReport() }),
    );
    const toggle = screen.getByRole("button");
    toggle.focus();
    await user.keyboard("{Enter}");
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    expect(screen.getByText("1 high")).toBeInTheDocument();
    expect(
      screen.getByText("1/2 stages completed · 3/2 files reviewed · 0 dropped"),
    ).toBeInTheDocument();
    expect(screen.getByText("deep").closest("[inert]")).not.toBeNull();
    const file = screen.getByRole("button", { name: "src/cache.ts:42-46" });
    expect(file.closest("[inert]")).not.toBeNull();
    const more = screen.getByRole("button", { name: "More details" });
    more.focus();
    await user.keyboard("{Enter}");
    expect(file.closest("[inert]")).toBeNull();
    await user.click(file);
    expect(openFile).toHaveBeenCalledWith({ path: "src/cache.ts", line: 42 });
    expect(screen.getByText(".refact/review_scratch/rv-1")).toBeInTheDocument();
    expect(container.querySelector("button button")).toBeNull();
  });

  test("zero findings is not a verified clean bill of health", () => {
    const report = reviewReport();
    report.findings = [];
    report.stages = [report.stages[0]];
    renderReview(
      message("report", { review_report: { ...report, outcome: "reviewed" } }),
    );
    expect(screen.getByText("0 findings reported")).toBeInTheDocument();
    expect(screen.getByRole("button")).not.toHaveTextContent(
      /success|no issues|verified/i,
    );
    fireEvent.click(screen.getByRole("button"));
    expect(
      screen.getByText("No supported findings; not a verification pass"),
    ).toBeInTheDocument();
  });

  test.each([
    { stages: [] },
    { stages: [{ status: "not_run" }] },
    { stages: [{ status: "failed" }] },
  ])(
    "missing or unfinished stages are unavailable, not a pass: %j",
    ({ stages }) => {
      renderReview(
        message("report", {
          review_report: {
            ...reviewReport(),
            outcome: "reviewed",
            findings: [],
            stages,
          },
        }),
      );
      expect(screen.getByText("Review unavailable")).toBeInTheDocument();
      expect(screen.getByRole("button")).not.toHaveTextContent(
        /success|no issues|verified/i,
      );
    },
  );

  test.each(["{malformed", "```json\n{broken}\n```", "```json\n[]\n```"])(
    "malformed evidence stays unavailable and recoverable: %s",
    (content) => {
      renderReview(message(content));
      expect(screen.getByText("Review unavailable")).toBeInTheDocument();
      openReview();
      expect(screen.getByText("Result")).toBeInTheDocument();
    },
  );

  test("pending never claims completion", () => {
    renderReview();
    expect(screen.getByRole("button")).toHaveTextContent("Awaiting review");
    expect(screen.getByRole("button")).not.toHaveTextContent(
      /completed|success|verified/i,
    );
  });

  test("reads fenced legacy typed reports", () => {
    renderReview(
      message(
        `Evidence\n\n\`\`\`json\n${JSON.stringify(reviewReport())}\n\`\`\``,
      ),
    );
    expect(screen.getByRole("button")).toHaveTextContent("2 findings");
    openReview();
    expect(screen.getByTestId("review-report")).toBeInTheDocument();
  });

  test.each([
    { state: "pending", result: undefined, raw: null },
    {
      state: "failed",
      result: message("# Failed\n\nNo diff available.", undefined, true),
      raw: "# Failed\n\nNo diff available.",
    },
    {
      state: "markdown",
      result: message("# Notes\n\n**Unverified** result."),
      raw: "# Notes\n\n**Unverified** result.",
    },
    {
      state: "malformed",
      result: message("```json\n{broken}\n```", {
        review_report: { invalid: "raw metadata" },
      }),
      raw: "```json\n{broken}\n```",
    },
    { state: "empty", result: message(""), raw: "" },
    {
      state: "structured",
      result: message("Original structured response", {
        review_report: reviewReport(),
      }),
      raw: "Original structured response",
    },
  ])(
    "preserves raw request and result in $state disclosure",
    ({ result, raw }) => {
      const args = '{"files":["src/cache.ts"],"focus":"Exact \\n request"}';
      const call = toolCall();
      call.function.arguments = args;
      const { container } = renderReview(result, call);
      fireEvent.click(screen.getByRole("button"));
      expect(container.querySelectorAll("dt").length).toBeLessThanOrEqual(3);
      const technical = screen.getByRole("button", {
        name: "Technical details",
      });
      expect(technical.closest("[inert]")).not.toBeNull();
      fireEvent.click(screen.getByRole("button", { name: "More details" }));
      expect(technical.closest("[inert]")).toBeNull();
      expect(technical).toHaveAttribute("aria-expanded", "false");
      fireEvent.click(technical);
      const codes = Array.from(container.querySelectorAll("code"));
      const request = codes.find((node) => node.textContent === args);
      expect(request).toBeDefined();
      expect(request?.closest("[inert]")).toBeNull();
      if (raw !== null) {
        expect(codes.some((node) => node.textContent === raw)).toBe(true);
      }
      if (result && "extra" in result && result.extra?.review_report) {
        expect(
          codes.some(
            (node) =>
              node.textContent ===
              JSON.stringify(result.extra?.review_report, null, 2),
          ),
        ).toBe(true);
      }
    },
  );

  test.each([
    { state: "empty stages", stages: [], outcome: "reviewed" },
    {
      state: "disabled stage",
      stages: [{ status: "not_run" }],
      outcome: "reviewed",
    },
    {
      state: "failed stage",
      stages: [{ status: "failed" }],
      outcome: "partial",
    },
    {
      state: "inconclusive",
      stages: reviewReport().stages,
      outcome: "inconclusive",
    },
  ])(
    "does not promote coverage in $state headline or first facts",
    ({ stages, outcome }) => {
      const { container } = renderReview(
        message("Unavailable report", {
          review_report: { ...reviewReport(), stages, outcome, findings: [] },
        }),
      );
      const toggle = screen.getByRole("button");
      expect(toggle).toHaveTextContent("2 files in scope");
      expect(toggle).toHaveTextContent("Review unavailable");
      expect(toggle).not.toHaveTextContent(
        /files reviewed|0 findings|no issues/i,
      );
      fireEvent.click(toggle);
      const coverage = container.querySelector("dl");
      expect(coverage).toHaveTextContent("2 files in scope");
      expect(coverage).not.toHaveTextContent("files reviewed");
    },
  );

  test("unavailable stages retain valid findings without claiming reviewed scope", () => {
    renderReview(
      message("Unavailable report with findings", {
        review_report: { ...reviewReport(), stages: [] },
      }),
    );
    expect(screen.getByRole("button")).toHaveTextContent(
      "2 files in scope · 2 findings reported",
    );
    expect(screen.getByRole("button")).toHaveTextContent("Review unavailable");
    openReview();
    expect(
      screen.getByText("Concurrent writes can overwrite a newer cache entry."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "src/cache.ts:42-46" }),
    ).toBeInTheDocument();
  });

  test("honors stored open state and keeps user collapse through streaming updates", () => {
    const values = new Map([["tc:review-card-1", true]]);
    const store = {
      get: (key: string) => values.get(key),
      set: (key: string, open: boolean) => values.set(key, open),
    };
    const view = (log: string[]) => (
      <CollapsibleStoreProvider value={store}>
        <CodeReviewTool toolCall={{ ...toolCall(), subchat_log: log }} />
      </CollapsibleStoreProvider>
    );
    const { rerender } = render(view([]));
    const toggle = screen.getByRole("button", { name: /Awaiting review/ });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    fireEvent.click(toggle);
    rerender(view(["new review progress"]));
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(values.get("tc:review-card-1")).toBe(false);
  });
});
