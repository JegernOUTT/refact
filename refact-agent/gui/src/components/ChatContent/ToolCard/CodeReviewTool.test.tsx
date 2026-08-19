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

function reportContent(): string {
  const report = {
    scope: {
      files_reviewed: ["src/cache.ts", "src/index.ts"],
      focus: "Concurrency correctness",
      diff_base: "main",
    },
    findings: [
      {
        id: "REV-1",
        category: "correctness",
        severity: "high",
        confidence: 0.94,
        verification_status: "verified",
        rank_tier: "execution_reproduced",
        sources: ["test", "trace"],
        file: "src/cache.ts",
        line1: 42,
        line2: 46,
        claim: "Concurrent writes can overwrite a newer cache entry.",
        evidence: [
          {
            kind: "execution",
            path: "src/cache.ts",
            line1: 42,
            line2: 46,
            content: "writerA();\nwriterB();",
          },
        ],
        impact: "Users can receive stale data.",
        remediation: "Serialize updates by key.",
        checks_performed: ["race reproduction"],
      },
    ],
    checks_performed: ["pnpm test cache"],
    summary: "One reproducible correctness issue.",
    assumed_intent: "Cache writes should preserve the latest value.",
    pipeline: {
      stages: [{ name: "mechanical", status: "completed", reason: null }],
      stopped_reason: null,
      mechanical: { passed: true, checks: [] },
      depth: "deep",
      agents: [
        {
          agent: "correctness-reviewer",
          model: "anthropic/claude-sonnet-review",
          status: "ran",
          candidates: 2,
          survived: 1,
          duration_ms: 1250,
          steps: 8,
        },
      ],
    },
  };
  return `# Review narrative\n\nStructured result follows.\n\n\`\`\`json\n${JSON.stringify(
    report,
  )}\n\`\`\``;
}

function message(content: string, failed = false): ChatMessage {
  return {
    role: "tool",
    tool_call_id: "review-card-1",
    content,
    tool_failed: failed,
  };
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
  test("renders a structured review report", () => {
    renderReview(message(reportContent()));

    expect(
      screen.getByText("Verdict: 1 execution-reproduced"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Concurrent writes can overwrite a newer cache entry."),
    ).toBeInTheDocument();
    expect(screen.getByText("execution-reproduced")).toBeInTheDocument();
    expect(screen.getAllByText("correctness-reviewer").length).toBeGreaterThan(
      0,
    );
    expect(screen.queryByText("Review narrative")).not.toBeInTheDocument();
  });

  test("falls back to markdown without a report block", () => {
    renderReview(message("# Review notes\n\nNo blocking issues found."));

    expect(screen.getByText("Review notes")).toBeInTheDocument();
    expect(screen.getByText("No blocking issues found.")).toBeInTheDocument();
  });

  test("shows error status when the tool failed", () => {
    const { container } = renderReview(
      message("# Review failed\n\nUnable to inspect the diff.", true),
    );

    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "error",
    );
  });
});
