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

function getExpandedStats() {
  const grids = screen.getAllByTestId("summarization-card-stats");
  return grids[grids.length - 1];
}

describe("SummarizationMessage", () => {
  it("renders deterministic tier as the default label", () => {
    render(
      <SummarizationMessage
        message={makeMessage({ summarization_tier: "tier0_deterministic" })}
      />,
    );
    expect(screen.getByTestId("summarization-card-tier")).toHaveTextContent(
      "Deterministic compaction",
    );
  });

  it("renders tier1_llm with LLM summary label", () => {
    render(
      <SummarizationMessage
        message={makeMessage({ summarization_tier: "tier1_llm" })}
      />,
    );
    expect(screen.getByTestId("summarization-card-tier")).toHaveTextContent(
      "LLM summary",
    );
  });

  it("renders tier1_merged with merged history summary label", () => {
    render(
      <SummarizationMessage
        message={makeMessage({ summarization_tier: "tier1_merged" })}
      />,
    );
    expect(screen.getByTestId("summarization-card-tier")).toHaveTextContent(
      "Merged history summary",
    );
  });

  it("renders tier2_reactive with reactive compaction label", () => {
    render(
      <SummarizationMessage
        message={makeMessage({ summarization_tier: "tier2_reactive" })}
      />,
    );
    expect(screen.getByTestId("summarization-card-tier")).toHaveTextContent(
      "Reactive compaction",
    );
  });

  it("renders an untyped summarization message as context compression", async () => {
    const messageWithoutTier: SummarizationMessageType = {
      role: "summarization",
      content: "no tier",
    };
    const { user } = render(
      <SummarizationMessage message={messageWithoutTier} />,
    );

    expect(screen.getByTestId("summarization-card-tier")).toHaveTextContent(
      "Context compression",
    );
    await user.click(screen.getByTestId("summarization-card-header"));
    expect(screen.getByText("no tier")).toBeInTheDocument();
  });

  it("shows the 1-based range in the header", () => {
    render(
      <SummarizationMessage
        message={makeMessage({
          summarized_range: [0, 5],
        })}
      />,
    );
    expect(screen.getByText(/messages 1–6/u)).toBeInTheDocument();
  });

  it("labels token estimate as 'saved' for tier2_reactive", () => {
    render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          summarized_token_estimate: 1234,
        })}
      />,
    );
    expect(screen.getByText(/1,234 tokens saved/u)).toBeInTheDocument();
  });

  it("labels token estimate as 'summarized' for tier1_llm", () => {
    render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier1_llm",
          summarized_token_estimate: 1234,
        })}
      />,
    );
    expect(screen.getByText(/1,234 tokens summarized/u)).toBeInTheDocument();
  });

  it("shows flattened assistant summary metadata in the header", () => {
    render(
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

    expect(screen.getByText("2 messages")).toBeInTheDocument();
    expect(screen.getByText(/summary-model/u)).toBeInTheDocument();
  });

  it("expands to reveal the body when the header is clicked", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({ content: "Important summary text" })}
      />,
    );
    expect(screen.queryByTestId("summarization-card-body")).toBeNull();
    await user.click(screen.getByTestId("summarization-card-header"));
    expect(screen.getByTestId("summarization-card-body")).toBeInTheDocument();
    expect(screen.getByText(/Important summary text/u)).toBeInTheDocument();
  });

  it("parses and displays per-stat cells for reactive compaction reports", async () => {
    const reactiveContent = [
      "## Reactive compaction report",
      "",
      "Context limit was reached, so compacted the conversation before retrying.",
      "",
      "- Attempt: 2",
      "- Context file entries deduplicated: 3",
      "- Tool outputs truncated: 5",
      "- Estimated tokens saved: 1024",
    ].join("\n");
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: reactiveContent,
        })}
      />,
    );
    await user.click(screen.getByTestId("summarization-card-header"));
    const stats = getExpandedStats();
    expect(stats).toHaveTextContent("Attempt");
    expect(stats).toHaveTextContent("2");
    expect(stats).toHaveTextContent("Context files deduped");
    expect(stats).toHaveTextContent("3");
    expect(stats).toHaveTextContent("Tool outputs truncated");
    expect(stats).toHaveTextContent("5");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("1024");
  });

  it("parses chat compression report stats", async () => {
    const reportContent = [
      "## Chat compression report",
      "",
      "- Context files removed: 2",
      "- Tool outputs truncated: 4",
      "- Tokens before: 10000",
      "- Tokens after: 7000",
      "- Estimated tokens saved: 3000",
      "- Reduction: 30%",
    ].join("\n");
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: reportContent,
        })}
      />,
    );

    await user.click(screen.getByTestId("summarization-card-header"));
    const stats = getExpandedStats();
    expect(stats).toHaveTextContent("Context files removed");
    expect(stats).toHaveTextContent("2");
    expect(stats).toHaveTextContent("Tool outputs truncated");
    expect(stats).toHaveTextContent("4");
    expect(stats).toHaveTextContent("Tokens before");
    expect(stats).toHaveTextContent("10000");
    expect(stats).toHaveTextContent("Tokens after");
    expect(stats).toHaveTextContent("7000");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("3000");
    expect(stats).toHaveTextContent("Reduction");
    expect(stats).toHaveTextContent("30%");
  });

  it("renders compression report metadata stats when content wording changes", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: "Compaction happened. Details moved to metadata.",
          extra: {
            compression_report: {
              kind: "chat_compression_report",
              context_files_removed: 2,
              tool_results_truncated: 4,
              tokens_before: 10000,
              tokens_after: 7000,
              estimated_tokens_saved: 3000,
              reduction_percent: 30,
            },
          },
        })}
      />,
    );

    await user.click(screen.getByTestId("summarization-card-header"));
    const stats = getExpandedStats();
    expect(stats).toHaveTextContent("Context files removed");
    expect(stats).toHaveTextContent("2");
    expect(stats).toHaveTextContent("Tool outputs truncated");
    expect(stats).toHaveTextContent("4");
    expect(stats).toHaveTextContent("Tokens before");
    expect(stats).toHaveTextContent("10,000");
    expect(stats).toHaveTextContent("Tokens after");
    expect(stats).toHaveTextContent("7,000");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("3,000");
    expect(stats).toHaveTextContent("Reduction");
    expect(stats).toHaveTextContent("30%");
  });

  it("renders flattened compression report metadata stats", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: "Compaction happened. Details moved to top-level metadata.",
          compression_report: {
            kind: "chat_compression_report",
            context_messages_dropped: 3,
            tokens_before: 10000,
            tokens_after: 7000,
            estimated_tokens_saved: 3000,
          },
        })}
      />,
    );

    await user.click(screen.getByTestId("summarization-card-header"));
    const stats = getExpandedStats();
    expect(stats).toHaveTextContent("Context messages dropped");
    expect(stats).toHaveTextContent("3");
    expect(stats).toHaveTextContent("Tokens before");
    expect(stats).toHaveTextContent("10,000");
    expect(stats).toHaveTextContent("Tokens after");
    expect(stats).toHaveTextContent("7,000");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("3,000");
  });

  it("compression_report_stats_include_llm_segment_summary_fields", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier1_llm",
          content: "## Chat context compressed\n\nSummary kept for the model.",
          compression_report: {
            kind: "chat_compression_report",
            compression_kind: "llm_segment_summary",
            source_message_count: 4,
            summary_model: "summary-model",
            tokens_before: 12000,
            tokens_after: 3000,
            estimated_tokens_saved: 9000,
            reduction_percent: 75,
          },
        })}
      />,
    );

    expect(screen.getByTestId("summarization-card-tier")).toHaveTextContent(
      "Context compressed",
    );
    await user.click(screen.getByTestId("summarization-card-header"));
    expect(
      screen.getByText(
        "Older context was summarized so this chat can continue within the model limit.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/Summary kept for the model/u)).toBeInTheDocument();
    const stats = getExpandedStats();
    expect(stats).toHaveTextContent("Messages compressed");
    expect(stats).toHaveTextContent("4");
    expect(stats).not.toHaveTextContent("Summary model");
    expect(stats).not.toHaveTextContent("summary-model");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("9,000");
    expect(stats).toHaveTextContent("Reduction");
    expect(stats).toHaveTextContent("75%");
  });

  it("renders llm segment reports as compact compression events", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier1_llm",
          content: "## Full report\n\nMarkdown details stay expandable.",
          compression_report: {
            kind: "chat_compression_report",
            compression_kind: "llm_segment_summary",
            source_message_count: 6,
            summary_model: "summary-model",
            tokens_before: 12000,
            tokens_after: 3000,
            estimated_tokens_saved: 9000,
            reduction_percent: 75,
          },
        })}
      />,
    );

    expect(screen.getByTestId("summarization-card-tier")).toHaveTextContent(
      "Context compressed",
    );
    expect(
      screen.getByText(
        "Older context was summarized so this chat can continue within the model limit.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Markdown details stay expandable/u)).toBeNull();

    const stats = screen.getByTestId("summarization-card-stats");
    expect(stats).toHaveTextContent("Messages compressed");
    expect(stats).toHaveTextContent("6");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("9,000");
    expect(stats).toHaveTextContent("Reduction");
    expect(stats).toHaveTextContent("75%");
    expect(stats).not.toHaveTextContent("Summary model");
    expect(stats).not.toHaveTextContent("summary-model");
    expect(stats).not.toHaveTextContent("Tokens before");

    await user.click(screen.getByTestId("summarization-card-header"));
    expect(
      screen.getByText(/Markdown details stay expandable/u),
    ).toBeInTheDocument();
    const expandedStats = getExpandedStats();
    expect(expandedStats).toHaveTextContent("Tokens before");
    expect(expandedStats).toHaveTextContent("12,000");
  });

  it("renders report markdown and paired model summary when expanded", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier1_llm",
          content: "## Full report\n\nReport details are visible.",
          paired_summary_content: "Paired model summary is visible.",
          compression_report: {
            kind: "chat_compression_report",
            compression_kind: "llm_segment_summary",
            source_message_count: 2,
            estimated_tokens_saved: 400,
          },
        })}
      />,
    );

    await user.click(screen.getByTestId("summarization-card-header"));
    expect(screen.getByText("Full report")).toBeInTheDocument();
    expect(screen.getByText(/Report details are visible/u)).toBeInTheDocument();
    expect(screen.getByText("Model summary")).toBeInTheDocument();
    expect(
      screen.getByText(/Paired model summary is visible/u),
    ).toBeInTheDocument();
  });

  it("legacy compression report copy does not claim original messages remain visible", () => {
    render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier1_llm",
          content: "## Legacy report\n\nThis markdown should stay collapsed.",
          compression_report: {
            kind: "chat_compression_report",
            compression_kind: "llm_segment_summary",
            source_message_count: 3,
            estimated_tokens_saved: 1200,
          },
        })}
      />,
    );

    expect(
      screen.getByText(
        /Older context was summarized so this chat can continue within the model limit/u,
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/Original messages remain visible/u),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "Messages compressed",
    );
  });

  it("compression report copy says original messages remain visible", () => {
    render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier1_llm",
          content: "## Verbose report\n\nThis markdown should stay collapsed.",
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

    expect(
      screen.getByText(/Original messages remain visible/u),
    ).toBeInTheDocument();
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "Messages summarized",
    );
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "3",
    );
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "Tokens saved",
    );
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "1,200",
    );
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "Context files preserved",
    );
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "2",
    );
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "Tool outputs compressed",
    );
    expect(screen.getByTestId("summarization-card-summary")).toHaveTextContent(
      "1",
    );
    expect(
      screen.queryByText(/This markdown should stay collapsed/u),
    ).toBeNull();
    expect(screen.queryByText(/replaced/iu)).toBeNull();
  });

  it("toggles expansion from the keyboard", async () => {
    const { user } = render(
      <SummarizationMessage message={makeMessage({ content: "details" })} />,
    );
    const header = screen.getByTestId("summarization-card-header");

    expect(header).toHaveAttribute("aria-expanded", "false");
    header.focus();
    await user.keyboard("{Enter}");
    expect(header).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByTestId("summarization-card-body")).toBeInTheDocument();

    await user.keyboard(" ");
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("summarization-card-body")).toBeNull();
  });

  it("renders context messages dropped from compression report metadata", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: "No parseable stat lines here.",
          extra: {
            compression_report: {
              kind: "chat_compression_report",
              context_messages_dropped: 3,
            },
          },
        })}
      />,
    );

    await user.click(screen.getByTestId("summarization-card-header"));
    const stats = getExpandedStats();
    expect(stats).toHaveTextContent("Context messages dropped");
    expect(stats).toHaveTextContent("3");
  });

  it("prefers compression report metadata over legacy markdown stats", async () => {
    const reportContent = [
      "## Chat compression report",
      "",
      "- Context files removed: 99",
      "- Estimated tokens saved: 99",
    ].join("\n");
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier2_reactive",
          content: reportContent,
          extra: {
            compression_report: {
              kind: "chat_compression_report",
              context_files_removed: 1,
              estimated_tokens_saved: 10,
            },
          },
        })}
      />,
    );

    await user.click(screen.getByTestId("summarization-card-header"));
    const stats = getExpandedStats();
    expect(stats).toHaveTextContent("Context files removed");
    expect(stats).toHaveTextContent("1");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("10");
    expect(stats).not.toHaveTextContent("99");
  });

  it("does not render a stats grid for non-reactive tiers", async () => {
    const { user } = render(
      <SummarizationMessage
        message={makeMessage({
          summarization_tier: "tier1_llm",
          content: "## Some heading\n\nProse body.",
        })}
      />,
    );
    await user.click(screen.getByTestId("summarization-card-header"));
    expect(screen.queryByTestId("summarization-card-stats")).toBeNull();
  });

  it("collapses again on a second click", async () => {
    const { user } = render(
      <SummarizationMessage message={makeMessage({ content: "details" })} />,
    );
    const header = screen.getByTestId("summarization-card-header");
    await user.click(header);
    expect(screen.getByTestId("summarization-card-body")).toBeInTheDocument();
    await user.click(header);
    expect(screen.queryByTestId("summarization-card-body")).toBeNull();
  });
});
