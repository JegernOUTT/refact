import { FileText } from "lucide-react";
import { describe, expect, test } from "vitest";

import {
  createDefaultChatState,
  render,
  screen,
} from "../../../utils/test-utils";
import type { ChatMessage, ToolCall } from "../../../services/refact/types";
import { ReportToolCard } from "./ReportToolCard";

function toolCall(id = "report-card-1"): ToolCall {
  return {
    id,
    index: 0,
    function: {
      name: "subagent",
      arguments: JSON.stringify({ task: "Investigate animation" }),
    },
    subchat_log: ["1/2: collecting context"],
  };
}

function toolMessage(id = "report-card-1"): ChatMessage {
  return {
    role: "tool",
    tool_call_id: id,
    content: "# Report\n\nAnimation fixed.",
  };
}

function renderReportTool(message?: ChatMessage) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.messages = message ? [message] : [];
  const id =
    message && "tool_call_id" in message
      ? message.tool_call_id
      : "report-card-1";

  return render(
    <ReportToolCard
      toolCall={toolCall(id)}
      icon={<FileText />}
      defaultSummary="Run subagent"
    />,
    { preloadedState: { chat } },
  );
}

describe("ReportToolCard", () => {
  test("routes its body through the shared animated collapsible wrapper", async () => {
    const { container, user } = renderReportTool(toolMessage());
    const card = container.querySelector("section");
    const toggle = screen.getByRole("button", { name: /run subagent/i });

    expect(card).toHaveAttribute("data-open", "true");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(container.querySelector(".rf-expand-grid")).toBeInTheDocument();
    expect(screen.getByText("Animation fixed.")).toBeInTheDocument();

    await user.click(toggle);

    expect(card).toHaveAttribute("data-open", "false");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
  });

  test("applies the running text shimmer and spinner while report tools are pending", () => {
    const { container } = renderReportTool();
    const toggle = screen.getByRole("button", { name: /run subagent/i });

    expect(container.querySelector("section")).toHaveAttribute(
      "data-status",
      "running",
    );
    expect(toggle).not.toHaveClass("rf-active-pulse");
    expect(toggle.querySelector(".rf-text-shimmer")).toHaveTextContent(
      "Run subagent",
    );
    expect(toggle.querySelector(".rf-spin")).toBeInTheDocument();
    expect(screen.getByText(/collecting context/)).toBeInTheDocument();
  });
});
