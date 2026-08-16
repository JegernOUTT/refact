import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test } from "vitest";

import { render } from "../../../utils/test-utils";
import { AriaSnapshotView } from "./AriaSnapshotView";

const SNAPSHOT = `- navigation "Primary":
  - link "Guide" [ref=e1]:
    - /url: /guide
  - heading "Controls" [level=2]
  - textbox "Search" [disabled] [expanded] [ref=e2]:
    - /placeholder: Find docs
  - checkbox "Subscribe" [checked]
- button "Save" [pressed=mixed] [selected] [ref=e3]`;

describe("AriaSnapshotView", () => {
  test("renders nested roles, names, states, refs, and properties", async () => {
    const user = userEvent.setup();
    render(<AriaSnapshotView yaml={SNAPSHOT} />);

    const view = screen.getByTestId("aria-snapshot-view");
    expect(within(view).getByText("navigation")).toBeInTheDocument();
    expect(within(view).getByText("“Primary”")).toBeInTheDocument();
    expect(within(view).getByText("level=2")).toBeInTheDocument();
    expect(within(view).getByText("disabled")).toBeInTheDocument();
    expect(within(view).getByText("expanded")).toBeInTheDocument();
    expect(within(view).getByText("checked")).toBeInTheDocument();
    expect(within(view).getByText("pressed=mixed")).toBeInTheDocument();
    expect(within(view).getByText("selected")).toBeInTheDocument();
    expect(within(view).getByText("/url: /guide")).toBeInTheDocument();
    expect(
      within(view).getByText("/placeholder: Find docs"),
    ).toBeInTheDocument();

    const toggle = within(view).getByRole("button", {
      name: "Collapse navigation Primary",
    });
    await user.click(toggle);
    expect(within(view).queryByText("“Guide”")).not.toBeInTheDocument();
    await user.click(toggle);
    expect(within(view).getByText("“Guide”")).toBeInTheDocument();
  });

  test("renders ref badges only for ref-bearing nodes and enriches flat metadata", () => {
    render(
      <AriaSnapshotView
        yaml={`- button "Save"\n- paragraph "Status"`}
        nodes={[
          { role: "button", name: "Save", ref: "e9" },
          { role: "paragraph", name: "Status", ref: null },
        ]}
      />,
    );

    const badges = screen.getAllByTestId("aria-ref-badge");
    expect(badges).toHaveLength(1);
    expect(badges[0]).toHaveTextContent("ref=e9");
    expect(screen.getByText("“Save”").closest("li")).toHaveAttribute(
      "data-has-ref",
      "true",
    );
    expect(screen.getByText("“Status”").closest("li")).toHaveAttribute(
      "data-has-ref",
      "false",
    );
  });

  test("filters to matching nodes and highlights the direct match", async () => {
    const user = userEvent.setup();
    render(<AriaSnapshotView yaml={SNAPSHOT} />);

    await user.type(
      screen.getByRole("textbox", { name: "Filter ARIA snapshot" }),
      "guide",
    );

    expect(screen.getByText("navigation")).toBeInTheDocument();
    expect(screen.getByText("“Guide”")).toBeInTheDocument();
    expect(screen.queryByText("“Search”")).not.toBeInTheDocument();
    expect(screen.queryByText("“Save”")).not.toBeInTheDocument();
    expect(screen.getByText("“Guide”").closest("[data-match]")).toHaveAttribute(
      "data-match",
      "true",
    );
  });

  test("limits large snapshots until expanded", async () => {
    const user = userEvent.setup();
    const yaml = Array.from(
      { length: 45 },
      (_, index) => `- button "Action ${index + 1}"`,
    ).join("\n");
    render(<AriaSnapshotView yaml={yaml} />);

    expect(screen.getByText("“Action 40”")).toBeInTheDocument();
    expect(screen.queryByText("“Action 41”")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Show 5 more nodes" }));
    expect(screen.getByText("“Action 45”")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Show 5 more nodes" }),
    ).not.toBeInTheDocument();
  });

  test("parses YAML-quoted node keys", () => {
    render(<AriaSnapshotView yaml={`- 'button "Save: changes" [ref=e4]'`} />);

    expect(screen.getByText("button")).toBeInTheDocument();
    expect(screen.getByText("“Save: changes”")).toBeInTheDocument();
    expect(screen.getByText("ref=e4")).toBeInTheDocument();
  });

  test("falls back to a code block for malformed YAML", () => {
    expect(() =>
      render(<AriaSnapshotView yaml={'- button "Save"\n   - link "Broken"'} />),
    ).not.toThrow();

    const fallback = screen.getByTestId("aria-snapshot-fallback");
    expect(fallback).toHaveTextContent('button "Save"');
    expect(fallback).toHaveTextContent('link "Broken"');
    expect(screen.queryByTestId("aria-snapshot-view")).not.toBeInTheDocument();
  });
});
