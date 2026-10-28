import { describe, it, expect } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { SummarizationMessage } from "./SummarizationMessage";
import type { SummarizationMessage as SummarizationMessageType } from "../../services/refact/types";

function makeMessage(
  overrides: Partial<SummarizationMessageType> = {},
): SummarizationMessageType {
  return {
    role: "summarization",
    content: "Summary body content",
    summarization_tier: "tier1_llm",
    ...overrides,
  };
}

const toggle = () => screen.getByTestId("transcript-toggle");
const stats = () => screen.getByTestId("summarization-card-stats");

describe("SummarizationMessage", () => {
  it("renders one collapsed Compressed · context row", () => {
    render(<SummarizationMessage message={makeMessage()} />);
    const card = screen.getByTestId("summarization-card");
    expect(card.getAttribute("data-transcript-item")).toBe("true");
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent(
      "Compressed",
    );
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "context",
    );
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("summarization-card-summary")).toBeNull();
  });

  it.each([
    ["tier0_deterministic", "Deterministic compaction"],
    ["tier1_llm", "LLM summary"],
    ["tier1_merged", "Merged history summary"],
    ["tier2_reactive", "Reactive compaction"],
    [undefined, "Context compression"],
  ] as const)("labels tier %s as %s in the summary", async (tier, label) => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({ summarization_tier: tier })}
      />,
    );
    await user.click(toggle());
    expect(stats()).toHaveTextContent(label);
  });

  it("puts message count and tokens saved in the meta", () => {
    render(
      <SummarizationMessage
        message={makeMessage({
          compression_report: {
            kind: "chat_compression_report",
            compression_kind: "llm_segment_summary",
            source_message_count: 1110,
            estimated_tokens_saved: 425000,
          },
        })}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "1,110 messages · −425k tokens",
    );
  });

  it("falls back to the range and token estimate in the meta", () => {
    render(
      <SummarizationMessage
        message={makeMessage({
          summarized_range: [0, 5],
          summarized_token_estimate: 1234,
        })}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "messages 1–6 · −1k tokens",
    );
  });

  it("shows assistant compression metadata", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          compression: {
            kind: "llm_segment_summary",
            source_message_ids: ["user-1", "assistant-1"],
            summary_model: "summary-model",
          },
        })}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "2 messages",
    );
    await user.click(toggle());
    expect(stats()).toHaveTextContent("Modelsummary-model");
  });

  it("shows the summary markdown inside a clamp when expanded", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          content: "Important summary text",
          paired_summary_content: "Paired model summary is visible",
        })}
      />,
    );
    expect(screen.queryByText(/Important summary text/u)).toBeNull();
    await user.click(toggle());
    expect(screen.getByTestId("transcript-clamp")).toBeInTheDocument();
    expect(screen.getByTestId("summarization-card-body")).toHaveTextContent(
      "Important summary text",
    );
    expect(screen.getByText("Model summary")).toBeInTheDocument();
    expect(
      screen.getByText(/Paired model summary is visible/u),
    ).toBeInTheDocument();
    await user.click(toggle());
    expect(toggle()).toHaveAttribute("aria-expanded", "false");
  });

  it("parses reactive compaction stats", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: [
            "- Attempt: 2",
            "- Context file entries deduplicated: 3",
            "- Tool outputs truncated: 5",
            "- Estimated tokens saved: 1024",
          ].join("\n"),
        })}
      />,
    );
    await user.click(toggle());
    expect(stats()).toHaveTextContent("Attempt2");
    expect(stats()).toHaveTextContent("Context files deduped3");
    expect(stats()).toHaveTextContent("Tool outputs truncated5");
    expect(stats()).toHaveTextContent("Tokens saved1024");
  });

  it("prefers compression report metadata over markdown stats", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: "- Context files removed: 99",
          extra: {
            compression_report: {
              kind: "chat_compression_report",
              context_files_removed: 2,
              tool_results_truncated: 4,
              tokens_before: 10000,
              tokens_after: 7000,
              estimated_tokens_saved: 3000,
              reduction_percent: 30,
              context_messages_dropped: 3,
            },
          },
        })}
      />,
    );
    await user.click(toggle());
    expect(stats()).not.toHaveTextContent("99");
    expect(stats()).toHaveTextContent("Context files removed2");
    expect(stats()).toHaveTextContent("Tool outputs truncated4");
    expect(stats()).toHaveTextContent("Tokens before10,000");
    expect(stats()).toHaveTextContent("Tokens after7,000");
    expect(stats()).toHaveTextContent("Reduction30%");
    expect(stats()).toHaveTextContent("Context messages dropped3");
  });

  it("does not render token stats for plain tiers", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({ summarization_tier: "tier1_llm" })}
      />,
    );
    await user.click(toggle());
    expect(stats()).not.toHaveTextContent("Tokens saved");
  });

  it("describes legacy segment reports without claiming originals remain", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          compression_report: {
            kind: "chat_compression_report",
            compression_kind: "llm_segment_summary",
            source_message_count: 3,
            estimated_tokens_saved: 1200,
          },
        })}
      />,
    );
    await user.click(toggle());
    expect(
      screen.getByText(/Older context was summarized/u),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Original messages remain visible/u)).toBeNull();
    expect(stats()).toHaveTextContent("Messages compressed3");
  });

  it("describes source-preserving reports", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          compression_report: {
            kind: "chat_compression_report",
            compression_kind: "llm_segment_summary",
            insert_mode: "source_preserving",
            source_message_count: 3,
            estimated_tokens_saved: 1200,
            preserved_context_file_count: 2,
            compressed_tool_output_count: 1,
          },
        })}
      />,
    );
    await user.click(toggle());
    expect(
      screen.getByText(/Original messages remain visible/u),
    ).toBeInTheDocument();
    expect(stats()).toHaveTextContent("Messages summarized3");
    expect(stats()).toHaveTextContent("Tokens saved1,200");
    expect(stats()).toHaveTextContent("Context files preserved2");
    expect(stats()).toHaveTextContent("Tool outputs compressed1");
  });
});
