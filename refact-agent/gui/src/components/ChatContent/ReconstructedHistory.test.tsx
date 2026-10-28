import { describe, it, expect } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { SummarizationMessage } from "./SummarizationMessage";
import { syntheticCompressionReportMessage } from "../../services/refact/types";

describe("reconstructed history disclosure", () => {
  it("retains flattened metadata and renders payload inside a collapsed report", async () => {
    const message = syntheticCompressionReportMessage({
      role: "compression_report",
      content: "Rebuilt",
      compression_report: {
        kind: "reconstructed_history",
        schema_version: 1,
        payload: {
          messages: [
            {
              role: "user",
              content: "Reconstructed request",
              message_id: "payload-1",
            },
          ],
        },
      },
    });
    const { user } = render(<SummarizationMessage message={message} />);
    const disclosure = screen.getByTestId("reconstructed-history-report");
    expect(disclosure).toHaveAttribute("data-open", "false");
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent(
      "Compressed",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "1 message",
    );
    expect(screen.queryByText("Reconstructed request")).toBeNull();
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(disclosure).toHaveAttribute("data-open", "true");
    expect(screen.getByTestId("summarization-card-stats")).toHaveTextContent(
      "Context rebuilt",
    );
    expect(screen.getAllByText("Reconstructed request")).toHaveLength(1);
  });

  it("shows rebuild metrics in the meta and trigger in the summary", async () => {
    const message = syntheticCompressionReportMessage({
      role: "compression_report",
      content: "Conversation context reconstructed.",
      compression_report: {
        kind: "reconstructed_history",
        schema_version: 1,
        model: "claude_code/claude-opus-5",
        trigger: "automatic",
        from_mode: "agent",
        to_mode: "agent",
        metrics: {
          messages_before: 41,
          messages_after: 6,
          tokens_before: 134486,
          tokens_after: 41590,
          estimated_tokens_saved: 92896,
          reduction_percent: 69,
        },
        payload: {
          messages: [
            {
              role: "user",
              content: "Reconstructed request",
              message_id: "payload-1",
            },
          ],
        },
      },
    });
    const { user } = render(<SummarizationMessage message={message} />);
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "41 → 6 messages · −93k tokens",
    );
    await user.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByText(/automatic at cap/)).toBeInTheDocument();
    expect(screen.getByText(/claude_code\/claude-opus-5/)).toBeInTheDocument();
    const stats = screen.getByTestId("summarization-card-stats");
    expect(stats).toHaveTextContent("Tokens before");
    expect(stats).toHaveTextContent("134,486");
    expect(stats).toHaveTextContent("Tokens after");
    expect(stats).toHaveTextContent("41,590");
    expect(stats).toHaveTextContent("69%");
    expect(screen.queryByText("Mode:", { exact: false })).toBeNull();
  });
});
