import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { VirtualList } from "./VirtualList";
import styles from "./VirtualList.module.css";

describe("VirtualList density", () => {
  it("preserves default item spacing and opts compact lists out explicitly", () => {
    const props = {
      items: ["First row"],
      renderItem: (item: string) => <span>{item}</span>,
    };
    const view = render(<VirtualList {...props} />);
    const item = () => screen.getByText("First row").parentElement;
    expect(item()).toHaveClass(styles.item);
    expect(item()).not.toHaveClass(styles.compactItem);

    view.rerender(<VirtualList {...props} density="compact" />);
    expect(item()).toHaveClass(styles.item, styles.compactItem);
    expect(view.container.firstChild).not.toHaveAttribute("density");

    view.rerender(<VirtualList {...props} />);
    expect(item()).not.toHaveClass(styles.compactItem);
  });
});
