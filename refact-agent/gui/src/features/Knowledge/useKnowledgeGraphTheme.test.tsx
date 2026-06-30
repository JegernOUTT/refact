import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import {
  useKnowledgeGraphTheme,
  type KnowledgeGraphColors,
} from "./useKnowledgeGraphTheme";

function Harness() {
  const { colors } = useKnowledgeGraphTheme();
  return <pre data-testid="colors">{JSON.stringify(colors)}</pre>;
}

function readColors(): KnowledgeGraphColors {
  return JSON.parse(
    screen.getByTestId("colors").textContent ?? "{}",
  ) as KnowledgeGraphColors;
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

  it("maps the dominant 'memory' kind and a visible default", () => {
    render(<Harness />);
    const colors = readColors();

    expect(typeof colors.kind.memory).toBe("string");
    expect(colors.kind.memory.length).toBeGreaterThan(0);
    expect(typeof colors.kindDefault).toBe("string");
    expect(colors.kindDefault.length).toBeGreaterThan(0);
    expect(typeof colors.foreground).toBe("string");
    expect(colors.foreground.length).toBeGreaterThan(0);
  });

  it("provides distinct semantic kinds beyond the legacy palette", () => {
    render(<Harness />);
    const colors = readColors();

    for (const kind of ["memory", "insight", "convention", "decision"]) {
      expect(
        colors.kind[kind],
        `missing color for kind "${kind}"`,
      ).toBeTruthy();
    }
  });
});
