import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import styles from "./Badge.module.css";
import { Badge } from "./Badge";

describe("Badge", () => {
  it("applies typed size and variant classes", () => {
    render(
      <Badge size="md" tone="warning" variant="glass">
        Review
      </Badge>,
    );

    const badge = screen.getByText("Review");

    expect(badge).toHaveClass(styles.badge);
    expect(badge).toHaveClass(styles["size-md"]);
    expect(badge).toHaveClass(styles["variant-glass"]);
    expect(badge).toHaveClass(styles.warning);
  });

  it("does not opt into interactive lift by default", () => {
    const { rerender } = render(<Badge>Info</Badge>);

    expect(screen.getByText("Info")).not.toHaveClass(styles.interactive);

    rerender(<Badge interactive>Info</Badge>);

    expect(screen.getByText("Info")).toHaveClass(styles.interactive);
  });
});
