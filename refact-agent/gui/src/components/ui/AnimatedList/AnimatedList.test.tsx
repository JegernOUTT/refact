import path from "node:path";
import { readFile } from "node:fs/promises";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import styles from "./AnimatedList.module.css";
import { AnimatedList } from "./AnimatedList";

describe("AnimatedList", () => {
  it("applies rf-stagger to the container and rise to initial children", () => {
    render(
      <AnimatedList data-testid="list">
        <div>First</div>
        <div>Second</div>
      </AnimatedList>,
    );

    const list = screen.getByTestId("list");
    expect(list).toHaveClass(styles.list);
    expect(list).toHaveClass("rf-stagger");
    expect(screen.getByText("First")).toHaveClass("rf-enter-rise");
    expect(screen.getByText("Second")).toHaveClass("rf-enter-rise");
  });

  it("can render semantic list elements", () => {
    render(
      <AnimatedList as="ul">
        <li>One</li>
        <li>Two</li>
      </AnimatedList>,
    );

    expect(screen.getByRole("list")).toHaveClass("rf-stagger");
    expect(screen.getByText("One")).toHaveClass("rf-enter-rise");
  });

  it("can disable stagger while keeping child entry motion", () => {
    render(
      <AnimatedList stagger={false} data-testid="list">
        <div>Only child</div>
      </AnimatedList>,
    );

    expect(screen.getByTestId("list")).not.toHaveClass("rf-stagger");
    expect(screen.getByText("Only child")).toHaveClass("rf-enter-rise");
  });

  it("limits initial child animation", () => {
    render(
      <AnimatedList initialItemLimit={1}>
        <div>Animated</div>
        <div>Static</div>
      </AnimatedList>,
    );

    expect(screen.getByText("Animated")).toHaveClass("rf-enter-rise");
    expect(screen.getByText("Static")).not.toHaveClass("rf-enter-rise");
  });

  it("keeps a reduced-motion CSS path", async () => {
    const css = await readFile(
      path.resolve(__dirname, "AnimatedList.module.css"),
      "utf8",
    );

    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation: none;");
    expect(css).toContain("transition: none;");
  });
});
