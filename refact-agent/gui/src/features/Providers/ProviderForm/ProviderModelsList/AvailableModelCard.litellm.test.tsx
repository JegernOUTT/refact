import { describe, expect, it } from "vitest";

import { render, screen } from "../../../../utils/test-utils";
import type { AvailableModel } from "../../../../services/refact";
import { AvailableModelCard } from "./AvailableModelCard";

const model: AvailableModel = {
  id: "litellm/claude-sonnet",
  display_name: "Claude Sonnet via LiteLLM",
  n_ctx: 200_000,
  max_output_tokens: 8_192,
  supports_tools: true,
  supports_parallel_tools: true,
  supports_strict_tools: true,
  supports_multimodality: true,
  supports_cache_control: true,
  supports_web_search: true,
  tokenizer: "cl100k_base",
  wire_format_override: "anthropic_messages",
  endpoint_override: "https://gateway.example.com/v1/messages",
  base_model: "claude-3-7-sonnet",
  upstream_provider: "anthropic",
  api_mode: "messages",
  supported_parameters: ["temperature", "tools", "web_search"],
  enabled: true,
  is_custom: false,
  pricing: { prompt: 3, generated: 15 },
};

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("LiteLLM AvailableModelCard metadata", () => {
  it("presents rich capabilities, routing identity, context, output, and pricing compactly", async () => {
    const { user } = render(
      <AvailableModelCard
        model={model}
        providerName="litellm_team"
        baseProvider="litellm"
        isReadonlyProvider
      />,
      { preloadedState },
    );

    expect(screen.getByText("Claude Sonnet via LiteLLM")).toBeInTheDocument();
    expect(screen.getByText("200K")).toBeInTheDocument();
    expect(screen.getByText("8K out")).toBeInTheDocument();
    expect(screen.getByText("$3.00/$15.00")).toBeInTheDocument();
    expect(screen.getByText("Parallel tools")).toBeInTheDocument();
    expect(screen.getByText("Strict/schema tools")).toBeInTheDocument();
    expect(screen.getByText("Cache")).toBeInTheDocument();
    expect(screen.getByText("Web search")).toBeInTheDocument();
    expect(
      screen.getByText("messages · anthropic_messages"),
    ).toBeInTheDocument();
    expect(screen.getByText("anthropic")).toBeInTheDocument();
    expect(screen.getByText("claude-3-7-sonnet")).toBeInTheDocument();
    expect(screen.getByText("cl100k_base")).toBeInTheDocument();

    await user.hover(screen.getByText("3 parameters"));
    expect(
      await screen.findAllByText(
        "Supported parameters: temperature, tools, web_search",
      ),
    ).not.toHaveLength(0);
  });
});
