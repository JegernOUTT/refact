import React from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { goodCaps, goodPing, server } from "../../utils/mockServer";
import { ModelSelector } from "./ModelSelector";

Element.prototype.hasPointerCapture = () => false;
Element.prototype.setPointerCapture = () => undefined;
Element.prototype.releasePointerCapture = () => undefined;

function renderSelector(
  props: Partial<React.ComponentProps<typeof ModelSelector>> = {},
) {
  server.use(goodPing, goodCaps);
  return render(
    <ModelSelector value={undefined} onValueChange={vi.fn()} {...props} />,
    {
      preloadedState: {
        config: {
          apiKey: "test",
          lspPort: 8001,
          themeProps: {},
          host: "vscode",
        },
      },
    },
  );
}

describe("ModelSelector", () => {
  it("keeps chat models as the default capability", async () => {
    const { user } = renderSelector({ compact: false });
    await user.click(await screen.findByRole("combobox"));

    expect(await screen.findAllByText("openai/gpt-4o")).not.toHaveLength(0);
    expect(screen.queryByText("openai/qwen2.5/coder/1.5b/base")).toBeNull();
    expect(screen.queryByText("openai/thenlper/gte-base")).toBeNull();
  });

  it("uses completion models for completion capability", async () => {
    const { user } = renderSelector({
      capability: "completion",
      compact: false,
    });
    await user.click(await screen.findByRole("combobox"));

    expect(
      await screen.findAllByText("openai/qwen2.5/coder/1.5b/base"),
    ).not.toHaveLength(0);
    expect(screen.queryByText("openai/o1")).toBeNull();
    expect(screen.queryByText("openai/thenlper/gte-base")).toBeNull();
    expect(
      screen.getAllByText("qwen2.5/coder/1.5b/base").length,
    ).toBeGreaterThan(0);
  });

  it("uses only the embedding model for embedding capability", async () => {
    const { user } = renderSelector({
      capability: "embedding",
      compact: false,
    });
    await user.click(await screen.findByRole("combobox"));

    expect(
      await screen.findAllByText("openai/text-embedding-3-small"),
    ).not.toHaveLength(0);
    expect(screen.queryByText("openai/gpt-4o")).toBeNull();
    expect(screen.queryByText("openai/qwen2.5/coder/1.5b/base")).toBeNull();
    expect(screen.getAllByText("1536 dims").length).toBeGreaterThan(0);
  });
});
