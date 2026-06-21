import React from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { goodCaps, goodPing, server } from "../../utils/mockServer";
import {
  ModelSamplingParams,
  type SamplingValues,
} from "./ModelSamplingParams";

function renderParams(props: {
  model: string;
  capability?: React.ComponentProps<typeof ModelSamplingParams>["capability"];
  values?: SamplingValues;
}) {
  server.use(goodPing, goodCaps);
  return render(
    <ModelSamplingParams
      model={props.model}
      capability={props.capability}
      values={props.values ?? {}}
      onChange={vi.fn()}
    />,
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

describe("ModelSamplingParams", () => {
  it("shows chat reasoning controls for reasoning-capable chat models", async () => {
    renderParams({ model: "openai/o1", capability: "chat" });

    expect(await screen.findByText("Reasoning")).toBeInTheDocument();
    expect(screen.getByText("Max tokens")).toBeInTheDocument();
  });

  it("uses completion metadata without chat reasoning controls", async () => {
    renderParams({
      model: "openai/qwen2.5/coder/1.5b/base",
      capability: "completion",
    });

    expect(await screen.findByText("Max tokens")).toBeInTheDocument();
    expect(screen.queryByText("Reasoning")).toBeNull();
    expect(screen.getByText("16384 (default)")).toBeInTheDocument();
  });

  it("hides sampling controls for embedding defaults", () => {
    renderParams({
      model: "openai/thenlper/gte-base",
      capability: "embedding",
    });

    expect(screen.queryByText("Max tokens")).toBeNull();
    expect(screen.queryByText("Reasoning")).toBeNull();
  });
});
