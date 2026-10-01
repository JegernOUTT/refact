import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import styles from "./Text.module.css";
import { Text } from "./Text";

describe("Text", () => {
  it.each([
    ["gray", "color-gray"],
    ["red", "color-red"],
    ["orange", "color-orange"],
    ["accent", "color-accent"],
  ] as const)("applies the %s color class", (color, className) => {
    render(<Text color={color}>Text content</Text>);

    const text = screen.getByText("Text content");
    expect(text).toHaveClass(styles[className]);
    for (const otherColor of ["gray", "red", "orange", "accent"] as const) {
      if (otherColor !== color) {
        expect(text).not.toHaveClass(styles[`color-${otherColor}`]);
      }
    }
  });

  it.each([
    ["left", "align-left"],
    ["center", "align-center"],
    ["right", "align-right"],
  ] as const)("applies the %s alignment class", (align, className) => {
    render(<Text align={align}>Text content</Text>);

    const text = screen.getByText("Text content");
    expect(text).toHaveClass(styles[className]);
    for (const otherAlign of ["left", "center", "right"] as const) {
      if (otherAlign !== align) {
        expect(text).not.toHaveClass(styles[`align-${otherAlign}`]);
      }
    }
  });

  it("does not apply a color class when color is omitted", () => {
    render(<Text>Text content</Text>);

    const text = screen.getByText("Text content");
    expect(text).not.toHaveClass(
      styles["color-gray"],
      styles["color-red"],
      styles["color-orange"],
      styles["color-accent"],
    );
  });
});
