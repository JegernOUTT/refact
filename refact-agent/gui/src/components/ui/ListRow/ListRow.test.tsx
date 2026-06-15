import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import styles from "./ListRow.module.css";
import { ListRow } from "./ListRow";

describe("ListRow", () => {
  it("renders all row slots", () => {
    render(
      <ListRow
        leading={<span data-testid="leading">L</span>}
        title="Primary title"
        subtitle="Secondary text"
        meta="Updated now"
        trailing={<button>Action</button>}
      />,
    );

    expect(screen.getByTestId("leading")).toBeInTheDocument();
    expect(screen.getByText("Primary title")).toBeInTheDocument();
    expect(screen.getByText("Secondary text")).toBeInTheDocument();
    expect(screen.getByText("Updated now")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Action" })).toBeInTheDocument();
  });

  it("renders as a real pressable button", () => {
    render(<ListRow as="button" title="Run action" />);

    const row = screen.getByRole("button", { name: "Run action" });
    expect(row).toHaveClass(styles.row);
    expect(row).toHaveClass(styles.interactive);
    expect(row).toHaveClass("rf-pressable");
  });

  it("applies the glass variant", () => {
    render(<ListRow variant="glass" title="Glass row" />);

    expect(screen.getByText("Glass row").closest("div")).toHaveClass(
      styles.glass,
    );
  });

  it("applies selected state as a row state", () => {
    render(<ListRow selected title="Selected row" />);

    const row = screen.getByText("Selected row").closest("div");
    expect(row).toHaveClass(styles.selected);
    expect(row).toHaveAttribute("data-selected", "true");
  });

  it("applies interactive and animated classes", () => {
    render(<ListRow interactive animated title="Animated interactive row" />);

    const row = screen.getByText("Animated interactive row").closest("div");
    expect(row).toHaveClass(styles.interactive);
    expect(row).toHaveClass("rf-pressable");
    expect(row).toHaveClass("rf-enter-rise");
  });
});
