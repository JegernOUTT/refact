import { describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen, waitFor } from "../../../../utils/test-utils";
import { server } from "../../../../utils/mockServer";
import { providersApi } from "../../../../services/refact";
import { AddCustomModelModal } from "./AddCustomModelModal";

const preloadedState = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

describe("AddCustomModelModal", () => {
  test("surfaces failed saves, keeps the modal open, and allows retry", async () => {
    let attempts = 0;

    server.use(
      http.post("*/v1/providers/openai_work/custom-models", () => {
        attempts += 1;
        if (attempts === 1) {
          return HttpResponse.json(
            { detail: "Custom model already exists." },
            { status: 500 },
          );
        }
        return HttpResponse.json({ success: true, model_id: "my-model" });
      }),
    );

    const onClose = vi.fn();
    const { user, store } = render(
      <AddCustomModelModal
        providerName="openai_work"
        isOpen
        onClose={onClose}
      />,
      { preloadedState },
    );

    await user.type(
      screen.getByPlaceholderText("e.g., my-custom-model"),
      "my-model",
    );
    await user.click(screen.getByRole("button", { name: "Add Model" }));

    expect(
      await screen.findByText("Custom model already exists."),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Custom model already exists.",
    );
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Add Model" }));

    await waitFor(() => expect(attempts).toBe(2));
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));

    store.dispatch(providersApi.util.resetApiState());
  });

  test("successful save preserves the custom model payload", async () => {
    let requestBody: unknown;

    server.use(
      http.post(
        "*/v1/providers/openai_work/custom-models",
        async ({ request }) => {
          requestBody = await request.json();
          return HttpResponse.json({ success: true, model_id: "my-model" });
        },
      ),
    );

    const onClose = vi.fn();
    const { user, store } = render(
      <AddCustomModelModal
        providerName="openai_work"
        isOpen
        onClose={onClose}
      />,
      { preloadedState },
    );

    await user.type(
      screen.getByPlaceholderText("e.g., my-custom-model"),
      "my-model",
    );
    await user.type(
      screen.getByPlaceholderText("low, medium, high"),
      "low, high",
    );
    await user.type(screen.getByPlaceholderText("e.g., 8192"), "2048");
    await user.click(screen.getByRole("button", { name: "Add Model" }));

    await waitFor(() => {
      expect(requestBody).toEqual(
        expect.objectContaining({
          id: "my-model",
          n_ctx: 4096,
          max_output_tokens: 2048,
          reasoning_effort_options: ["low", "high"],
          pricing: null,
        }),
      );
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    store.dispatch(providersApi.util.resetApiState());
  });
});
