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

  it("keeps rows and search on one content-width contract without gutter hacks", async () => {
    const css = await readFile(
      path.resolve(__dirname, "ModelSelector.module.css"),
      "utf8",
    );
    const scrollArea = css.match(/\.scrollArea \{[^}]+\}/)?.[0] ?? "";
    const row = css.match(/\.row \{[^}]+\}/)?.[0] ?? "";

    expect(scrollArea).toContain("overflow-y: auto;");
    expect(scrollArea).not.toContain("padding-right");
    expect(scrollArea).not.toContain("margin-right");
    expect(row).toContain("width: 100%;");
  });
});
