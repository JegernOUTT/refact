import { describe, expect, it } from "vitest";
import { render } from "../../utils/test-utils";
import { Markdown } from "./Markdown";
import styles from "./Markdown.module.css";

describe("Markdown tables", () => {
  it("renders each table inside a horizontal scroll wrapper", () => {
    const { container } = render(
      <Markdown>
        {"| Priority | Status |\n| --- | --- |\n| Critical | Open |"}
      </Markdown>,
    );

    const table = container.querySelector("table");
    expect(table).not.toBeNull();
    expect(table?.parentElement).toHaveClass(styles.tableScroll);
    expect(table).toHaveClass(styles.table);
  });
});
