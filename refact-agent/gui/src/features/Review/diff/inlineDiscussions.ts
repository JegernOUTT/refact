import { StateEffect, StateField, type Extension } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  WidgetType,
  gutter,
  GutterMarker,
  keymap,
} from "@codemirror/view";

export type InlineHost = { id: string; endLine: number; host: HTMLElement };
export const setInlineHosts = StateEffect.define<readonly InlineHost[]>();
const observers = new WeakMap<HTMLElement, ResizeObserver>();

class DiscussionWidget extends WidgetType {
  constructor(readonly item: InlineHost) {
    super();
  }
  eq(other: DiscussionWidget) {
    return this.item.host === other.item.host;
  }
  toDOM(view: EditorView) {
    const mount = document.createElement("div");
    mount.className = "rf-review-inline-mount";
    mount.append(this.item.host);
    if (this.item.host.dataset.focusPending === "true") {
      queueMicrotask(() => {
        if (!this.item.host.isConnected) return;
        delete this.item.host.dataset.focusPending;
        this.item.host.querySelector<HTMLTextAreaElement>("textarea")?.focus();
      });
    }
    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(() => {
        if (observers.get(mount) === observer) view.requestMeasure();
      });
      observers.set(mount, observer);
      observer.observe(this.item.host);
    }
    return mount;
  }
  destroy(dom: HTMLElement) {
    observers.get(dom)?.disconnect();
    observers.delete(dom);
  }
  ignoreEvent() {
    return true;
  }
}

const hostsField = StateField.define<readonly InlineHost[]>({
  create: () => [],
  update(value, transaction) {
    for (const effect of transaction.effects)
      if (effect.is(setInlineHosts)) return effect.value;
    return value;
  },
  provide: (field) =>
    EditorView.decorations.from(field, (items) =>
      Decoration.set(
        items.map((item) =>
          Decoration.widget({
            widget: new DiscussionWidget(item),
            block: true,
            side: 1,
          }).range(item.endLine),
        ),
        true,
      ),
    ),
});

class CommentMarker extends GutterMarker {
  constructor(
    readonly activate: (line: number) => void,
    readonly line: number,
  ) {
    super();
  }
  eq(other: CommentMarker) {
    return this.line === other.line && this.activate === other.activate;
  }
  toDOM() {
    const button = document.createElement("button");
    button.type = "button";
    // The editor shortcut provides keyboard access without adding one tab
    // stop per (normally hidden) gutter marker.
    button.tabIndex = -1;
    button.className = "rf-review-comment-line";
    button.textContent = "+";
    button.setAttribute("aria-label", `Comment on line ${this.line}`);
    button.onclick = () => this.activate(this.line);
    return button;
  }
}

export function inlineDiscussionExtension(
  activate: (line: number) => void,
): Extension {
  return [
    hostsField,
    EditorView.contentAttributes.of({
      "aria-description": "Press Control Alt M to comment on the cursor line.",
    }),
    keymap.of([
      {
        key: "Ctrl-Alt-m",
        run: (view) => {
          activate(
            view.state.doc.lineAt(view.state.selection.main.head).number,
          );
          return true;
        },
      },
    ]),
    gutter({
      class: "rf-review-comment-gutter",
      lineMarker: (view, line) =>
        new CommentMarker(activate, view.state.doc.lineAt(line.from).number),
    }),
  ];
}

export function applyInlineHosts(
  view: EditorView,
  items: readonly InlineHost[],
) {
  const valid = items
    .filter((item) => item.endLine >= 1 && item.endLine <= view.state.doc.lines)
    .map((item) => ({
      ...item,
      endLine: view.state.doc.line(item.endLine).to,
    }));
  const previous = view.state.field(hostsField);
  if (
    previous.length === valid.length &&
    previous.every(
      (item, index) =>
        item.id === valid[index].id &&
        item.endLine === valid[index].endLine &&
        item.host === valid[index].host,
    )
  )
    return;
  view.dispatch({ effects: setInlineHosts.of(valid) });
}
