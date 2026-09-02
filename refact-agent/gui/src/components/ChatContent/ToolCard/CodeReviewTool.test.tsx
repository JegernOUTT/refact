import { describe, expect, test } from "vitest";
import {
  createDefaultChatState,
  render,
  screen,
} from "../../../utils/test-utils";
import type { ChatMessage, ToolCall } from "../../../services/refact/types";
import { CodeReviewTool } from "./CodeReviewTool";

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

function renderReview(messageValue: ChatMessage) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.messages = [messageValue];
  return render(<CodeReviewTool toolCall={toolCall()} />, {
    preloadedState: { chat },
  });
}

describe("CodeReviewTool", () => {
  test("renders the structured report from tool metering", () => {
    renderReview(
      message("## Review · 2 file(s) requested", {
        review_report: reviewReport(),
      }),
    );

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

  test("falls back to markdown without a report", () => {
    renderReview(message("# Review notes\n\nNo blocking issues found."));

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
  });
});
