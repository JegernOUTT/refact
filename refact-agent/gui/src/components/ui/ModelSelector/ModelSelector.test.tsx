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

  it("paints selected and hover rows through the reserved scrollbar gutter", async () => {
    const css = await readFile(
      path.resolve(__dirname, "ModelSelector.module.css"),
      "utf8",
    );
    const listRoot = css.match(/\.listRoot \{[^}]+\}/)?.[0] ?? "";
    const row = css.match(/\.row \{[^}]+\}/)?.[0] ?? "";
    const rowPaint = css.match(/\.row::before \{[^}]+\}/)?.[0] ?? "";
    const rowHover =
      css.match(/\.row:hover:not\(:disabled\)::before \{[^}]+\}/)?.[0] ?? "";
    const rowSelected =
      css.match(/\.row\[data-selected="true"\]::before \{[^}]+\}/)?.[0] ?? "";
    const rowContent = css.match(/\.rowContent \{[^}]+\}/)?.[0] ?? "";

    expect(listRoot).toContain(
      "--rf-model-selector-row-paint-outset-inline-end: calc(",
    );
    expect(listRoot).toContain("var(--rf-scrollbar-size, 8px)");
    expect(listRoot).toContain("2 * var(--rf-hairline)");
    expect(row).toContain("position: relative;");
    expect(row).toContain("background: transparent;");
    expect(rowPaint).toContain("inset-inline: 0");
    expect(rowPaint).toContain(
      "calc(-1 * var(--rf-model-selector-row-paint-outset-inline-end));",
    );
    expect(rowPaint).toContain("pointer-events: none;");
    expect(rowHover).toContain("background: var(--rf-surface-1);");
    expect(rowSelected).toContain("background: var(--rf-color-accent-soft);");
    expect(rowContent).toContain("z-index: 1;");
  });
});
