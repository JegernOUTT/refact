import { readFile } from "node:fs/promises";
import path from "node:path";

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ModelSelector } from "./ModelSelector";
import type { ModelOption } from "./ModelSelector";
import styles from "./ModelSelector.module.css";

const models: ModelOption[] = [
  {
    value: "openai/gpt-5.5",
    displayName: "OpenAI GPT 5.5 Ultra Long Model Name For Truncation",
    pricing: { prompt: "$1.25", output: "$10.00" },
    contextWindow: "400K ctx",
    badges: ["default", "reasoning"] as const,
  },
  {
    value: "anthropic/claude-sonnet-4.5",
    displayName: "Claude Sonnet 4.5",
    badges: ["task-agent"] as const,
  },
];

describe("ModelSelector", () => {
  it("marks the selected row while preserving inline list semantics", () => {
    render(
      <ModelSelector
        models={models}
        value="openai/gpt-5.5"
        variant="inline"
        onSelect={vi.fn()}
      />,
    );

    const listbox = screen.getByRole("listbox", { name: "Models" });
    const selected = screen.getByRole("option", {
      name: /OpenAI GPT 5\.5 Ultra Long Model Name For Truncation/i,
    });

    expect(listbox).toHaveClass(styles.scrollArea);
    expect(selected).toHaveClass(styles.row);
    expect(selected).toHaveAttribute("aria-selected", "true");
    expect(selected).toHaveAttribute("data-selected", "true");
  });

  it("keeps the scroll gutter outside the painted row width", async () => {
    const css = await readFile(
      path.resolve(__dirname, "ModelSelector.module.css"),
      "utf8",
    );

    expect(css).toContain("padding-right: var(--rf-space-2);");
    expect(css).toContain("margin-right: calc(-1 * var(--rf-space-2));");
    expect(css).toContain(".row {");
    expect(css).toContain("width: 100%;");
  });
});
