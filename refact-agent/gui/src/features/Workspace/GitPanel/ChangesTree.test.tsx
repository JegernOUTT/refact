import { fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { render, screen } from "../../../utils/test-utils";
import { ChangesTree, type ChangesTreeProps } from "./ChangesTree";
import { makeEntry, type VisibleChangesRow } from "./changesTreeModel";
import listStyles from "../../../components/ui/VirtualList/VirtualList.module.css";

function treeProps(count: number): ChangesTreeProps {
  const rows: VisibleChangesRow[] = Array.from(
    { length: count },
    (_, index) => {
      const entry = makeEntry(
        "change",
        `src/deep/path/file-${index}.tsx`,
        `/repo/file-${index}.tsx`,
        "MODIFIED",
        null,
      );
      return {
        node: { type: "file", id: entry.id, group: "changes", entry },
        depth: 1,
        parentId: null,
      };
    },
  );
  return {
    rows,
    collapsed: new Set(),
    selectedId: rows[0].node.id,
    inclusion: { excludedChanges: new Set(), includedUnversioned: new Set() },
    groupByDirectory: false,
    onSelect: vi.fn(),
    onSetExpanded: vi.fn(),
    onToggleInclude: vi.fn(),
    onShowDiff: vi.fn(),
    renderContextMenu: () => null,
  };
}

describe("ChangesTree rendering", () => {
  it("opts into compact item wrappers only above 200 visible rows and retains keyboard actions", () => {
    const props = treeProps(201);
    const view = render(
      <ChangesTree {...props} rows={props.rows.slice(0, 200)} />,
    );
    expect(
      view.container.querySelector(`.${listStyles.compactItem}`),
    ).toBeNull();
    view.rerender(<ChangesTree {...props} />);
    const row = screen.getByRole("treeitem", {
      name: "src/deep/path/file-0.tsx",
    });
    expect(row.parentElement).toHaveClass(listStyles.compactItem);
    const tree = screen.getByRole("tree");
    fireEvent.keyDown(tree, { key: "End" });
    expect(props.onSelect).toHaveBeenLastCalledWith(props.rows[200].node.id);
    fireEvent.keyDown(tree, { key: " " });
    expect(props.onToggleInclude).toHaveBeenCalledWith(props.rows[0].node);
    fireEvent.keyDown(tree, { key: "Enter" });
    const first = props.rows[0].node;
    if (first.type !== "file") throw new Error("expected file");
    expect(props.onShowDiff).toHaveBeenCalledWith(first.entry);
  });

  it("keeps full destination and rename origin in the tooltip and accessible path", () => {
    const props = treeProps(1);
    const node = props.rows[0].node;
    if (node.type !== "file") throw new Error("expected file");
    node.entry.status = "RENAMED";
    node.entry.oldPath = "previous/old-name.tsx";
    render(<ChangesTree {...props} />);
    const row = screen.getByRole("treeitem", { name: node.entry.path });
    expect(row).toHaveAttribute(
      "title",
      `${node.entry.path} (renamed from previous/old-name.tsx)`,
    );
    expect(row).toHaveAttribute("aria-level", "2");
    expect(
      screen.getByRole("checkbox", {
        name: `Include ${node.entry.path} in commit`,
      }),
    ).toBeChecked();
  });
});
