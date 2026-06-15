import { describe, it, expect, vi } from "vitest";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { render } from "../../utils/test-utils";
import { MemoryTagFilter } from "./MemoryTagFilter";

const TAGS = ["rust", "backend", "architecture", "style"];

function noop() {
  /* intentionally empty */
}

describe("MemoryTagFilter", () => {
  it("renders nothing when there are no tags", () => {
    render(
      <MemoryTagFilter
        allTags={[]}
        selectedTags={new Set()}
        onToggleTag={noop}
        onClearTags={noop}
      />,
    );

    expect(
      screen.queryByRole("button", { name: /Filter by tag/i }),
    ).not.toBeInTheDocument();
  });

  it("shows selected tags as chips and a clear control", async () => {
    const user = userEvent.setup();
    const onToggleTag = vi.fn();
    const onClearTags = vi.fn();

    render(
      <MemoryTagFilter
        allTags={TAGS}
        selectedTags={new Set(["rust"])}
        onToggleTag={onToggleTag}
        onClearTags={onClearTags}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: "Remove rust filter" }),
    );
    expect(onToggleTag).toHaveBeenCalledWith("rust");

    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(onClearTags).toHaveBeenCalled();
  });

  it("filters options by search and toggles a tag from the popover", async () => {
    const user = userEvent.setup();
    const onToggleTag = vi.fn();

    render(
      <MemoryTagFilter
        allTags={TAGS}
        selectedTags={new Set()}
        onToggleTag={onToggleTag}
        onClearTags={noop}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Filter by tag/i }));

    const search = await screen.findByRole("textbox", { name: "Search tags" });
    await user.type(search, "arch");

    expect(
      screen.getByRole("button", { name: "architecture" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "backend" }),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "architecture" }));
    expect(onToggleTag).toHaveBeenCalledWith("architecture");
  });
});
