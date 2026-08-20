import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import styles from "./CardGrid.module.css";
import { CardGrid } from "./CardGrid";

describe("CardGrid", () => {
  it("renders children inside the grid wrapper", () => {
    render(
      <CardGrid data-testid="grid">
        <div>First card</div>
        <div>Second card</div>
      </CardGrid>,
    );

    const grid = screen.getByTestId("grid");

    expect(grid).toHaveClass(styles.grid);
    expect(grid).toHaveClass(styles.regular);
    expect(grid).not.toHaveClass(styles.dense);
    expect(screen.getByText("First card").parentElement).toBe(grid);
    expect(screen.getByText("Second card").parentElement).toBe(grid);
  });

  it("toggles the dense track class", () => {
    render(
      <CardGrid dense data-testid="grid">
        <div>Card</div>
      </CardGrid>,
    );

    const grid = screen.getByTestId("grid");

    expect(grid).toHaveClass(styles.dense);
    expect(grid).not.toHaveClass(styles.regular);
  });
});
