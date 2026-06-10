import { readFile } from "node:fs/promises";
import path from "node:path";

import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";

import { render } from "../../../utils/test-utils";
import { Tabs } from "./Tabs";

describe("Tabs", () => {
  it("preserves caller list styles while setting tab indicator variables", () => {
    render(
      <Tabs defaultValue="two">
        <Tabs.List activeIndex={1} style={{ marginTop: "12px" }}>
          <Tabs.Trigger value="one">One</Tabs.Trigger>
          <Tabs.Trigger value="two">Two</Tabs.Trigger>
        </Tabs.List>
      </Tabs>,
    );

    expect(screen.getByRole("tablist")).toHaveStyle({
      marginTop: "12px",
      "--rf-tabs-count": "2",
      "--rf-tabs-index": "1",
    });
  });

  it("does not reserve a global scrollbar gutter in tab strips", async () => {
    const css = await readFile(
      path.resolve(__dirname, "Tabs.module.css"),
      "utf8",
    );
    const list = css.match(/\.list \{[^}]+\}/)?.[0] ?? "";
    const trigger = css.match(/\.trigger \{[^}]+\}/)?.[0] ?? "";

    expect(list).toContain("display: grid;");
    expect(list).toContain("scrollbar-gutter: auto;");
    expect(list).toContain("overflow: auto hidden;");
    expect(trigger).toContain("scrollbar-gutter: auto;");
  });

  it("renders an empty tab list without an indicator", () => {
    const { container } = render(
      <Tabs defaultValue="missing">
        <Tabs.List aria-label="Empty tabs" />
      </Tabs>,
    );

    expect(screen.getByRole("tablist", { name: "Empty tabs" })).toHaveStyle({
      "--rf-tabs-count": "1",
      "--rf-tabs-index": "0",
    });
    expect(container.querySelector("span[aria-hidden='true']")).toBeNull();
  });
});
