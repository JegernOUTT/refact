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
    const stats = screen.getByTestId("summarization-card-stats");
    expect(stats).toHaveTextContent("Attempt");
    expect(stats).toHaveTextContent("2");
    expect(stats).toHaveTextContent("Context files deduped");
    expect(stats).toHaveTextContent("3");
    expect(stats).toHaveTextContent("Tool outputs truncated");
    expect(stats).toHaveTextContent("5");
    expect(stats).toHaveTextContent("Tokens saved");
    expect(stats).toHaveTextContent("1024");
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
