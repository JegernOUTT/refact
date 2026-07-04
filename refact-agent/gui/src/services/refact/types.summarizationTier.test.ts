import { describe, expect, it } from "vitest";
import {
  syntheticCompressionReportMessage,
  syntheticSummarizationMessage,
} from "./types";
import type { AssistantMessage, CompressionReportMessage } from "./types";

describe("synthetic summarization tier normalization", () => {
  const assistantSummary = (summarization_tier?: string): AssistantMessage => ({
    role: "assistant",
    content: "summary body",
    message_id: "m-1",
    ...(summarization_tier === undefined ? {} : { summarization_tier }),
  });

  it("maps the engine summary kind onto tier1_llm", () => {
    expect(
      syntheticSummarizationMessage(assistantSummary("llm_segment_summary"))
        .summarization_tier,
    ).toBe("tier1_llm");
  });

  it("passes known tiers through instead of hardcoding tier1_llm", () => {
    expect(
      syntheticSummarizationMessage(assistantSummary("tier1_merged"))
        .summarization_tier,
    ).toBe("tier1_merged");
    expect(
      syntheticSummarizationMessage(assistantSummary("tier0_deterministic"))
        .summarization_tier,
    ).toBe("tier0_deterministic");
  });

  it("falls back to tier1_llm when the tier is missing", () => {
    expect(
      syntheticSummarizationMessage(assistantSummary()).summarization_tier,
    ).toBe("tier1_llm");
  });

  it("keeps report tier passthrough with a reactive fallback", () => {
    const report = (
      summarization_tier?: CompressionReportMessage["summarization_tier"],
    ): CompressionReportMessage => ({
      role: "compression_report",
      content: "report body",
      message_id: "r-1",
      ...(summarization_tier === undefined ? {} : { summarization_tier }),
    });

    expect(
      syntheticCompressionReportMessage(report("tier1_llm")).summarization_tier,
    ).toBe("tier1_llm");
    expect(syntheticCompressionReportMessage(report()).summarization_tier).toBe(
      "tier2_reactive",
    );
  });
});
