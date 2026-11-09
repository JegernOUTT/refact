import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";

import { render } from "../../../utils/test-utils";
import * as sharedHost from "../../Workspace/GitPanel/diff/DiffEditorsHost";
import * as editors from "../../Workspace/GitPanel/diff/diffEditors";
import * as language from "../../Workspace/GitPanel/diff/diffLanguage";
import gitStyles from "../../Workspace/GitPanel/diff/GitDiffViewer.module.css";
import styles from "../Review.module.css";
import {
  ReviewDiffEditors,
  type ReviewDiffEditorsProps,
} from "./ReviewDiffEditors";

afterEach(() => {
  vi.restoreAllMocks();
});

const props: ReviewDiffEditorsProps = {
  mode: "side-by-side",
  left: "old\nkeep\nlast\n",
  right: "new\nkeep\nlast\n",
  path: "snapshot.txt",
  leftLabel: "Captured base",
  rightLabel: "Captured head",
  highlights: {
    left: [],
    right: [
      { id: "f1", kind: "finding", startLine: 1, endLine: 1, active: true },
    ],
  },
  reveal: null,
  onSelection: () => undefined,
};

test("uses the Git host for immutable review highlights, selection and focus-free reveal without rebuilding", () => {
  const host = vi.spyOn(sharedHost, "DiffEditorsHost");
  const create = vi.spyOn(editors, "createDiffEditors");
  const onSelection = vi.fn();
  const { rerender } = render(
    <ReviewDiffEditors {...props} onSelection={onSelection} />,
  );
  expect(host).toHaveBeenCalled();
  expect(create).toHaveBeenCalledTimes(1);
  expect(screen.getByTestId("review-diff-editors")).toHaveClass(
    gitStyles.host,
    styles.diffHost,
  );
  const controller = create.mock.results[0]
    .value as editors.DiffEditorsController;
  const { a, b } = controller.views();
  expect(a?.state.doc.toString()).toBe(props.left);
  expect(b.state.doc.toString()).toBe(props.right);
  expect(b.state.facet(EditorState.readOnly)).toBe(true);
  expect(b.dom.querySelector(".rf-review-hl-finding")).not.toBeNull();
  const focus = vi.spyOn(EditorView.prototype, "focus");
  b.dispatch({ selection: { anchor: 0, head: 8 } });
  expect(onSelection).toHaveBeenLastCalledWith("right", {
    startLine: 1,
    endLine: 2,
  });
  const selection = b.state.selection;
  const nextSelection = vi.fn();
  rerender(
    <ReviewDiffEditors
      {...props}
      highlights={{
        left: [
          { id: "t1", kind: "thread", startLine: 2, endLine: 2, active: true },
        ],
        right: [],
      }}
      reveal={{ pane: "left", line: 2, token: 1 }}
      onSelection={nextSelection}
    />,
  );
  expect(create).toHaveBeenCalledTimes(1);
  expect(controller.views().b).toBe(b);
  expect(b.state.selection).toBe(selection);
  expect(a?.dom.querySelector(".rf-review-hl-thread")).not.toBeNull();
  expect(b.dom.querySelector(".rf-review-hl-finding")).toBeNull();
  expect(focus).not.toHaveBeenCalled();
  a?.dispatch({ selection: { anchor: 0, head: 3 } });
  expect(nextSelection).toHaveBeenLastCalledWith("left", {
    startLine: 1,
    endLine: 1,
  });
});

test("unified populated head retains right-side selection and comment identity", () => {
  const create = vi.spyOn(editors, "createDiffEditors");
  const onSelection = vi.fn();
  const onCommentLine = vi.fn();
  render(
    <ReviewDiffEditors
      {...props}
      mode="unified"
      left=""
      onSelection={onSelection}
      onCommentLine={onCommentLine}
    />,
  );
  const controller = create.mock.results[0]
    .value as editors.DiffEditorsController;
  const { a, b } = controller.views();
  expect(a).toBeNull();
  b.dispatch({ selection: { anchor: 0, head: 3 } });
  expect(onSelection).toHaveBeenLastCalledWith("right", {
    startLine: 1,
    endLine: 1,
  });
  const button = b.dom.querySelector<HTMLButtonElement>(
    '[aria-label="Comment on line 1"]',
  );
  expect(button?.tabIndex).toBe(-1);
  button?.click();
  expect(onCommentLine).toHaveBeenCalledWith("right", 1);
});

test("initializes highlights and reveal after host rebuilds and ignores stale language loads on teardown", async () => {
  const pending: ((support: null) => void)[] = [];
  vi.spyOn(language, "resolveLanguageForPath").mockImplementation(
    () => new Promise((resolve) => pending.push(resolve)),
  );
  const create = vi.spyOn(editors, "createDiffEditors");
  const scroll = vi.spyOn(EditorView, "scrollIntoView");
  const focus = vi.spyOn(EditorView.prototype, "focus");
  const reveal = { pane: "left" as const, line: 2, token: 1 };
  const { rerender, unmount } = render(
    <ReviewDiffEditors {...props} reveal={reveal} />,
  );
  const first = create.mock.results[0].value as editors.DiffEditorsController;
  const staleLanguage = vi.spyOn(first, "setLanguage");
  const destroyFirst = vi.spyOn(first, "destroy");
  rerender(
    <ReviewDiffEditors
      {...props}
      mode="unified"
      path="other.txt"
      reveal={reveal}
    />,
  );
  expect(create).toHaveBeenCalledTimes(2);
  const second = create.mock.results[1].value as editors.DiffEditorsController;
  expect(destroyFirst).toHaveBeenCalledOnce();
  expect(
    second.views().b.dom.querySelector(".rf-review-hl-finding"),
  ).not.toBeNull();
  expect(scroll).toHaveBeenLastCalledWith(4, { y: "center" });
  expect(focus).not.toHaveBeenCalled();
  const currentLanguage = vi.spyOn(second, "setLanguage");
  const destroySecond = vi.spyOn(second, "destroy");
  pending[0](null);
  await waitFor(() => expect(pending).toHaveLength(2));
  expect(staleLanguage).not.toHaveBeenCalled();
  expect(currentLanguage).not.toHaveBeenCalled();
  unmount();
  pending[1](null);
  await Promise.resolve();
  expect(currentLanguage).not.toHaveBeenCalled();
  expect(destroySecond).toHaveBeenCalledOnce();
});
