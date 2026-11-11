import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { screen, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
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

test.each(["unified", "side-by-side"] as const)(
  "%s visits the first changed hunk, traverses deletion/addition and restores keyed snapshots",
  async (mode) => {
    const create = vi.spyOn(editors, "createDiffEditors");
    const scroll = vi.spyOn(EditorView, "scrollIntoView");
    const prefix =
      Array.from({ length: 40 }, (_, i) => `unchanged ${i}`).join("\n") + "\n";
    const middle =
      Array.from({ length: 40 }, (_, i) => `context ${i}`).join("\n") + "\n";
    const positions = new Map<
      string,
      import("./ReviewDiffEditors").DiffPosition
    >();
    const input = {
      ...props,
      mode,
      left: prefix + "deleted\n" + middle,
      right: prefix + middle + "added\n",
      positions,
    };
    const { rerender } = render(
      <ReviewDiffEditors {...input} navigationKey="candidate:full:a:mode" />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("1 / 2 changes");
    expect(scroll).toHaveBeenCalledWith(prefix.length, { y: "center" });
    screen.getByRole("button", { name: "Next change" }).click();
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("2 / 2 changes"),
    );
    expect(screen.getByRole("button", { name: "Next change" })).toBeDisabled();
    const first = create.mock.results[0].value as editors.DiffEditorsController;
    const snapshot = vi.spyOn(first.views().b, "scrollSnapshot");
    first.views().b.scrollDOM.scrollTop = 10000;
    first.views().b.scrollDOM.dispatchEvent(new Event("scroll"));
    rerender(
      <ReviewDiffEditors
        key="other"
        {...props}
        positions={positions}
        navigationKey="candidate:full:b:mode"
      />,
    );
    expect(snapshot).toHaveBeenCalled();
    expect(positions.has("candidate:full:a:mode")).toBe(true);
    rerender(
      <ReviewDiffEditors
        key="original"
        {...input}
        navigationKey="candidate:full:a:mode"
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("2 / 2 changes");
    screen.getByRole("button", { name: "Previous change" }).click();
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("1 / 2 changes"),
    );
  },
);

test("an explicit reveal overrides cached scroll, but streaming highlights never replay it", () => {
  const create = vi.spyOn(editors, "createDiffEditors");
  const scroll = vi.spyOn(EditorView, "scrollIntoView");
  const positions = new Map<
    string,
    import("./ReviewDiffEditors").DiffPosition
  >();
  const { rerender } = render(
    <ReviewDiffEditors
      {...props}
      positions={positions}
      navigationKey="a"
      reveal={{ pane: "right", line: 2, token: 1 }}
    />,
  );
  rerender(
    <ReviewDiffEditors
      key="b"
      {...props}
      positions={positions}
      navigationKey="b"
    />,
  );
  scroll.mockClear();
  rerender(
    <ReviewDiffEditors
      key="a"
      {...props}
      positions={positions}
      navigationKey="a"
      reveal={{ pane: "right", line: 3, token: 2 }}
    />,
  );
  expect(scroll).toHaveBeenLastCalledWith(9, { y: "center" });
  const builds = create.mock.calls.length;
  scroll.mockClear();
  rerender(
    <ReviewDiffEditors
      key="a"
      {...props}
      positions={positions}
      navigationKey="a"
      reveal={{ pane: "right", line: 3, token: 2 }}
      highlights={{
        left: [],
        right: [
          {
            id: "streamed",
            kind: "finding",
            startLine: 1,
            endLine: 1,
            active: false,
          },
        ],
      }}
    />,
  );
  expect(scroll).not.toHaveBeenCalled();
  expect(create).toHaveBeenCalledTimes(builds);
});

test("initial explicit reveal selects the matching hunk on mount", () => {
  const context =
    Array.from({ length: 40 }, (_, i) => `context ${i}`).join("\n") + "\n";
  render(
    <ReviewDiffEditors
      {...props}
      left={"old\n" + context + "removed\n"}
      right={"new\n" + context + "added\n"}
      reveal={{ pane: "right", line: 42, token: 9 }}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent("2 / 2 changes");
  expect(screen.getByRole("button", { name: "Next change" })).toBeDisabled();
});

test.each(["a", "b"] as const)(
  "split scroll from %s mirrors only vertical movement and skips equal positions",
  (side) => {
    const create = vi.spyOn(editors, "createDiffEditors");
    render(<ReviewDiffEditors {...props} autoRevealFirstChange={false} />);
    const controller = create.mock.results[0]
      .value as editors.DiffEditorsController;
    const { a, b } = controller.views();
    if (!a) throw new Error("Expected split editors");
    const source = side === "a" ? a : b;
    const target = side === "a" ? b : a;
    target.scrollDOM.scrollLeft = 25;
    let targetTop = 0;
    const write = vi.fn((value: number) => {
      targetTop = value;
    });
    Object.defineProperty(target.scrollDOM, "scrollTop", {
      configurable: true,
      get: () => targetTop,
      set: write,
    });
    source.scrollDOM.scrollTop = 120;
    source.scrollDOM.dispatchEvent(new Event("scroll"));
    expect(target.scrollDOM.scrollTop).toBe(120);
    expect(target.scrollDOM.scrollLeft).toBe(25);
    expect(write).toHaveBeenCalledTimes(1);
    target.scrollDOM.dispatchEvent(new Event("scroll"));
    source.scrollDOM.dispatchEvent(new Event("scroll"));
    expect(write).toHaveBeenCalledTimes(1);
    const sourceLine = vi.spyOn(source, "lineBlockAtHeight");
    const targetLine = vi.spyOn(target, "lineBlockAtHeight");
    source.contentDOM.dispatchEvent(new Event("wheel"));
    source.scrollDOM.scrollTop = 160;
    source.scrollDOM.dispatchEvent(new Event("scroll"));
    target.scrollDOM.dispatchEvent(new Event("scroll"));
    expect(sourceLine).toHaveBeenCalledOnce();
    expect(targetLine).not.toHaveBeenCalled();
  },
);

test("a mirror clamped by the other pane does not pull the scrolled pane back", () => {
  const create = vi.spyOn(editors, "createDiffEditors");
  render(<ReviewDiffEditors {...props} autoRevealFirstChange={false} />);
  const controller = create.mock.results[0]
    .value as editors.DiffEditorsController;
  const { a, b } = controller.views();
  if (!a) throw new Error("Expected split editors");
  let baseTop = 0;
  Object.defineProperty(a.scrollDOM, "scrollTop", {
    configurable: true,
    get: () => baseTop,
    set: (value: number) => {
      baseTop = Math.min(value, 100);
    },
  });
  b.scrollDOM.scrollTop = 115;
  b.scrollDOM.dispatchEvent(new Event("scroll"));
  expect(a.scrollDOM.scrollTop).toBe(100);
  a.scrollDOM.dispatchEvent(new Event("scroll"));
  expect(b.scrollDOM.scrollTop).toBe(115);
});

test("unified scroll remains local", () => {
  const create = vi.spyOn(editors, "createDiffEditors");
  render(<ReviewDiffEditors {...props} mode="unified" />);
  const controller = create.mock.results[0]
    .value as editors.DiffEditorsController;
  const { a, b } = controller.views();
  expect(a).toBeNull();
  b.scrollDOM.scrollTop = 120;
  const write = vi.fn();
  Object.defineProperty(b.scrollDOM, "scrollTop", {
    configurable: true,
    get: () => 120,
    set: write,
  });
  b.scrollDOM.dispatchEvent(new Event("scroll"));
  expect(b.scrollDOM.scrollTop).toBe(120);
  expect(write).not.toHaveBeenCalled();
});

test("split restore prefers the cached head position over a divergent base", async () => {
  const create = vi.spyOn(editors, "createDiffEditors");
  const positions = new Map<
    string,
    import("./ReviewDiffEditors").DiffPosition
  >();
  const { rerender } = render(
    <ReviewDiffEditors {...props} positions={positions} navigationKey="a" />,
  );
  const first = create.mock.results[0].value as editors.DiffEditorsController;
  first.views().b.scrollDOM.dispatchEvent(new Event("scroll"));
  rerender(
    <ReviewDiffEditors
      key="b"
      {...props}
      positions={positions}
      navigationKey="b"
    />,
  );
  const cached = positions.get("a");
  if (!cached) throw new Error("Expected cached position");
  positions.set("a", { ...cached, topA: 80, topB: 240 });
  rerender(
    <ReviewDiffEditors
      key="a"
      {...props}
      positions={positions}
      navigationKey="a"
    />,
  );
  const controller = create.mock.results[2]
    .value as editors.DiffEditorsController;
  await waitFor(() => {
    expect(controller.views().a?.scrollDOM.scrollTop).toBe(240);
    expect(controller.views().b.scrollDOM.scrollTop).toBe(240);
  });
});

function pendingRestoration() {
  const create = vi.spyOn(editors, "createDiffEditors");
  const positions = new Map<
    string,
    import("./ReviewDiffEditors").DiffPosition
  >();
  const input = {
    ...props,
    left: "old\n" + "context\n".repeat(40) + "removed\n",
    right: "new\n" + "context\n".repeat(40) + "added\n",
  };
  const rendered = render(
    <ReviewDiffEditors {...input} positions={positions} navigationKey="a" />,
  );
  const first = create.mock.results[0].value as editors.DiffEditorsController;
  positions.set("a", {
    b: first.views().b.scrollSnapshot(),
    topB: 240,
    chunk: 0,
  });
  const frames = new Map<number, FrameRequestCallback>();
  let id = 0;
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
    frames.set(++id, callback);
    return id;
  });
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation((key) => {
    frames.delete(key);
  });
  rendered.rerender(
    <ReviewDiffEditors
      key="restored"
      {...input}
      positions={positions}
      navigationKey="a"
    />,
  );
  const controller = create.mock.results[1]
    .value as editors.DiffEditorsController;
  const frame = () => {
    const pending = [...frames.values()];
    frames.clear();
    act(() => pending.forEach((callback) => callback(0)));
  };
  return { ...rendered, controller, positions, frame, frames };
}

test("restores absolute offsets after late measured anchor movement without caching transient scroll", () => {
  const { controller, positions, frame, frames } = pendingRestoration();
  const { a, b } = controller.views();
  const measure = vi.spyOn(b, "lineBlockAtHeight");
  measure.mockImplementationOnce(() => {
    b.scrollDOM.scrollTop = 260;
    return { from: 0 } as ReturnType<EditorView["lineBlockAtHeight"]>;
  });
  frame();
  expect(measure).toHaveBeenCalledWith(240);
  expect(b.scrollDOM.scrollTop).toBe(240);
  b.scrollDOM.scrollTop = 260;
  b.scrollDOM.dispatchEvent(new Event("scroll"));
  expect(positions.get("a")?.topB).toBe(240);
  expect(a?.scrollDOM.scrollTop).toBe(240);
  frame();
  expect(b.scrollDOM.scrollTop).toBe(240);
  for (let i = 0; i < 4; i++) frame();
  expect(a?.scrollDOM.scrollTop).toBe(240);
  expect(b.scrollDOM.scrollTop).toBe(240);
  expect(positions.get("a")?.topB).toBe(240);
  expect(frames.size).toBe(0);
});

test.each([
  "wheel",
  "pointerdown",
  "touchstart",
  "keydown",
  "scroller-wheel",
  "scroller-pointerdown",
  "scroller-touchstart",
])("%s cancels pending restoration and preserves the user's scroll", (type) => {
  const { controller, positions, frame } = pendingRestoration();
  const { b } = controller.views();
  const event =
    type === "keydown"
      ? new KeyboardEvent(type, { key: "PageDown", bubbles: true })
      : new Event(type.replace("scroller-", ""), { bubbles: true });
  if (type === "touchstart")
    Object.defineProperty(event, "targetTouches", { value: [] });
  (type.startsWith("scroller-") ? b.scrollDOM : b.contentDOM).dispatchEvent(
    event,
  );
  b.scrollDOM.scrollTop = 310;
  b.scrollDOM.dispatchEvent(new Event("scroll"));
  for (let i = 0; i < 5; i++) frame();
  expect(b.scrollDOM.scrollTop).toBe(310);
  expect(positions.get("a")?.topB).toBe(310);
});

test.each(["reveal", "hunk", "unmount", "replacement"] as const)(
  "%s cancels pending restoration before it can overwrite newer navigation",
  (action) => {
    const { controller, positions, frame, rerender, unmount } =
      pendingRestoration();
    const { b } = controller.views();
    const measured = vi.spyOn(b, "lineBlockAtHeight");
    if (action === "unmount") unmount();
    else if (action === "hunk")
      screen.getByRole("button", { name: "Next change" }).click();
    else
      rerender(
        <ReviewDiffEditors
          key="restored"
          {...props}
          positions={positions}
          navigationKey={action === "replacement" ? "b" : "a"}
          path={action === "replacement" ? "other.txt" : props.path}
          reveal={
            action === "reveal" ? { pane: "right", line: 3, token: 1 } : null
          }
        />,
      );
    measured.mockClear();
    for (let i = 0; i < 5; i++) frame();
    expect(measured).not.toHaveBeenCalled();
    if (action === "unmount" || action === "replacement")
      expect(positions.get("a")?.topB).toBe(240);
  },
);

test("bounded restoration preserves the cached offset when hidden panes clamp scrolling", () => {
  const { controller, positions, frame, frames } = pendingRestoration();
  for (const view of [controller.views().a, controller.views().b]) {
    if (view)
      Object.defineProperty(view.scrollDOM, "scrollTop", {
        configurable: true,
        get: () => 0,
        set: () => undefined,
      });
  }
  for (let i = 0; i < 15; i++) frame();
  expect(frames.size).toBe(0);
  expect(positions.get("a")?.topB).toBe(240);
});

test("queued clamped restoration scroll after timeout cannot replace the desired cache", () => {
  const { controller, positions, frame, frames } = pendingRestoration();
  let limit = 100;
  for (const view of [controller.views().a, controller.views().b]) {
    if (!view) continue;
    let top = 0;
    Object.defineProperty(view.scrollDOM, "scrollTop", {
      configurable: true,
      get: () => top,
      set: (value: number) => {
        top = Math.min(value, limit);
      },
    });
  }
  for (let i = 0; i < 11; i++) frame();
  limit = 180;
  frame();
  for (let i = 0; i < 3; i++) frame();
  expect(frames.size).toBe(0);
  const { b } = controller.views();
  expect(b.scrollDOM.scrollTop).toBe(180);
  b.scrollDOM.dispatchEvent(new Event("scroll"));
  expect(positions.get("a")?.topB).toBe(240);
  b.scrollDOM.dispatchEvent(new Event("wheel"));
  b.scrollDOM.scrollTop = 160;
  b.scrollDOM.dispatchEvent(new Event("scroll"));
  expect(positions.get("a")?.topB).toBe(160);
});
