import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen, waitFor } from "../../../../utils/test-utils";
import { server } from "../../../../utils/mockServer";
import { AddCustomModelModal } from "./AddCustomModelModal";
import { providersApi } from "../../../../services/refact";

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("AddCustomModelModal", () => {
  it("keeps chat custom model creation payload unchanged", async () => {
    let requestBody: unknown;
    server.use(
      http.post(
        "*/v1/providers/custom_work/custom-models",
        async ({ request }) => {
          requestBody = await request.json();
          return HttpResponse.json({ success: true });
        },
      ),
    );

    const { user, store } = render(
      <AddCustomModelModal
        providerName="custom_work"
        isOpen
        onClose={() => undefined}
      />,
      { preloadedState },
    );

    await user.type(
      screen.getByPlaceholderText("e.g., my-custom-model"),
      "chat-model",
    );
    await user.clear(screen.getByPlaceholderText("4096"));
    await user.type(screen.getByPlaceholderText("4096"), "32000");
    await user.click(screen.getByText("Supports Tools (function calling)"));
    await user.type(
      screen.getByPlaceholderText("hf://Xenova/claude-tokenizer"),
      "hf://Tokenizer",
    );
    await user.click(screen.getByRole("button", { name: "Add Model" }));

    await waitFor(() => {
      expect(requestBody).toEqual({
        id: "chat-model",
        n_ctx: 32000,
        supports_tools: true,
        supports_multimodality: false,
        supports_thinking_budget: false,
        supports_adaptive_thinking_budget: false,
        supports_cache_control: true,
        reasoning_effort_options: null,
        tokenizer: "hf://Tokenizer",
        max_output_tokens: undefined,
        pricing: null,
      });
    });
    store.dispatch(providersApi.util.resetApiState());
  });
});
