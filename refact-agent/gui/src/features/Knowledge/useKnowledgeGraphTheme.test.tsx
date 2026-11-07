import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { useKnowledgeGraphTheme } from "./useKnowledgeGraphTheme";

function Harness() {
  const { colors } = useKnowledgeGraphTheme();
  return <pre data-testid="colors">{JSON.stringify(colors)}</pre>;
}

afterEach(() => {
  cleanup();
});

describe("useKnowledgeGraphTheme", () => {
  it("only exposes concrete colors cytoscape can paint", () => {
    render(<Harness />);
    const text = screen.getByTestId("colors").textContent ?? "";

    // Raw CSS custom-property indirection cannot be painted on a canvas.
    expect(text).not.toContain("var(");
    expect(text).not.toContain("color-mix(");
    expect(text.length).toBeGreaterThan(2);
  });
});
