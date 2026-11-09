import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { afterEach, expect, test, vi } from "vitest";
import {
  createDiffEditors,
  type DiffEditorsController,
} from "../../Workspace/GitPanel/diff/diffEditors";
import {
  applyInlineHosts,
  inlineDiscussionExtension,
} from "./inlineDiscussions";

const controllers: DiffEditorsController[] = [];
afterEach(() => {
  controllers.splice(0).forEach((controller) => controller.destroy());
  vi.restoreAllMocks();
});

function editor(mode: "side-by-side" | "unified") {
  const parent = document.createElement("div");
  document.body.append(parent);
  const controller = createDiffEditors({
    parent,
    mode,
    left: "old\nkeep\nend",
    right: "new\nkeep\nend",
    collapseUnchanged: false,
    isDark: false,
    language: null,
    leftLabel: "Base",
    rightLabel: "Head",
    extensions: {
      a: inlineDiscussionExtension(() => undefined),
      b: inlineDiscussionExtension(() => undefined),
    },
  });
  controllers.push(controller);
  return controller.views();
}

for (const mode of ["side-by-side", "unified"] as const)
  test(`${mode} mounts independent coincident discussions after the exact end line without focus`, () => {
    const views = editor(mode);
    const focus = vi.spyOn(EditorView.prototype, "focus");
    for (const view of [views.a, views.b]) {
      if (!view) continue;
      const first = document.createElement("div");
      first.textContent = "First thread";
      const second = document.createElement("div");
      second.textContent = "Second thread";
      applyInlineHosts(view, [
        { id: "one", endLine: 2, host: first },
        { id: "two", endLine: 2, host: second },
      ]);
      const positions: number[] = [];
      for (const decorations of view.state.facet(EditorView.decorations)) {
        if (typeof decorations !== "function")
          decorations.between(
            0,
            view.state.doc.length,
            (from, _to, decoration) => {
              const spec = decoration.spec as { widget?: { item?: unknown } };
              if (spec.widget?.item) positions.push(from);
            },
          );
      }
      expect(positions).toEqual([
        view.state.doc.line(2).to,
        view.state.doc.line(2).to,
      ]);
      expect(view.contentDOM.contains(first)).toBe(true);
      expect(view.contentDOM.contains(second)).toBe(true);
      expect(first.parentElement?.nextSibling).toBe(second.parentElement);
      const original = first;
      applyInlineHosts(view, [{ id: "one", endLine: 2, host: first }]);
      expect(view.contentDOM.contains(original)).toBe(true);
      expect(second.isConnected).toBe(false);
      applyInlineHosts(view, []);
      expect(first.isConnected).toBe(false);
      expect(view.state.facet(EditorState.readOnly)).toBe(true);
    }
    expect(focus).not.toHaveBeenCalled();
  });

test("measures resizing and disconnects widget observers on removal", () => {
  const observe = vi.fn();
  const disconnect = vi.fn();
  let resized: (() => void) | undefined;
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resized = callback;
      }
      observe = observe;
      disconnect = disconnect;
    },
  );
  const { b } = editor("unified");
  const measure = vi.spyOn(b, "requestMeasure");
  const host = document.createElement("div");
  applyInlineHosts(b, [{ id: "resize", endLine: 1, host }]);
  applyInlineHosts(b, [{ id: "resize", endLine: 1, host }]);
  expect(
    observe.mock.calls.filter(([element]) => element === host),
  ).toHaveLength(1);
  expect(observe).toHaveBeenCalledWith(host);
  measure.mockClear();
  resized?.();
  expect(measure).toHaveBeenCalled();
  applyInlineHosts(b, []);
  expect(disconnect).toHaveBeenCalled();
  measure.mockClear();
  resized?.();
  expect(measure).not.toHaveBeenCalled();
  b.destroy();
  vi.unstubAllGlobals();
});

test("relocating a stable portal host cleans each mount observer independently", () => {
  const records: {
    callback: () => void;
    disconnect: ReturnType<typeof vi.fn>;
    element?: Element;
  }[] = [];
  vi.stubGlobal(
    "ResizeObserver",
    class {
      private record;
      constructor(callback: () => void) {
        this.record = {
          callback,
          disconnect: vi.fn(),
          element: undefined as Element | undefined,
        };
        records.push(this.record);
      }
      observe(element: Element) {
        this.record.element = element;
      }
      disconnect() {
        this.record.disconnect();
      }
    },
  );
  const parent = document.createElement("div");
  document.body.append(parent);
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: Array.from({ length: 200 }, (_, i) => `line ${i}`).join("\n"),
      extensions: inlineDiscussionExtension(() => undefined),
    }),
  });
  const host = document.createElement("div");
  const measure = vi.spyOn(view, "requestMeasure");
  applyInlineHosts(view, [{ id: "moving", endLine: 1, host }]);
  applyInlineHosts(view, [{ id: "moving", endLine: 200, host }]);
  view.dispatch({
    effects: EditorView.scrollIntoView(view.state.doc.line(200).from),
  });
  applyInlineHosts(view, [{ id: "moving", endLine: 1, host }]);
  view.dispatch({ effects: EditorView.scrollIntoView(0) });
  applyInlineHosts(view, [{ id: "moving", endLine: 1, host }]);
  const mounts = records.filter((record) => record.element === host);
  expect(mounts.length).toBeGreaterThan(0);
  for (const record of mounts) {
    measure.mockClear();
    record.callback();
    if (record.disconnect.mock.calls.length)
      expect(measure).not.toHaveBeenCalled();
  }
  applyInlineHosts(view, []);
  view.destroy();
  for (const record of mounts) {
    expect(record.disconnect).toHaveBeenCalledTimes(1);
    measure.mockClear();
    record.callback();
    expect(measure).not.toHaveBeenCalled();
  }
  vi.unstubAllGlobals();
});

test("accessible cursor command comments without a range selection", () => {
  const activate = vi.fn();
  const parent = document.createElement("div");
  document.body.append(parent);
  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: "one\ntwo",
      extensions: inlineDiscussionExtension(activate),
    }),
  });
  view.dispatch({ selection: { anchor: 5 } });
  view.contentDOM.dispatchEvent(
    new KeyboardEvent("keydown", {
      key: "m",
      code: "KeyM",
      ctrlKey: true,
      altKey: true,
      bubbles: true,
    }),
  );
  expect(activate).toHaveBeenCalledWith(2);
  expect(view.contentDOM).toHaveAttribute(
    "aria-description",
    expect.stringContaining("Control Alt M"),
  );
  view.destroy();
});
