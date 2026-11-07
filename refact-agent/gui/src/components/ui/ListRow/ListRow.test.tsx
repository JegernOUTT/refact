import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ListRow } from "./ListRow";
import styles from "./ListRow.module.css";

describe("ListRow", () => {
  it("renders as a real pressable button", () => {
    render(<ListRow as="button" title="Run action" />);

    const row = screen.getByRole("button", { name: "Run action" });
    expect(row).toHaveClass(styles.row);
    expect(row).toHaveClass(styles.interactive);
    expect(row).toHaveClass("rf-pressable");
  });
});
