import {
  RangeSetBuilder,
  StateEffect,
  StateField,
  type Extension,
} from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView } from "@codemirror/view";

export type HighlightKind = "finding" | "explanation" | "thread" | "tour";

export type LineHighlight = {
  id: string;
  kind: HighlightKind;
  startLine: number;
  endLine: number;
  active: boolean;
};

export const HIGHLIGHT_CLASS: Record<HighlightKind, string> = {
  finding: "rf-review-hl-finding",
  explanation: "rf-review-hl-explanation",
  thread: "rf-review-hl-thread",
  tour: "rf-review-hl-tour",
};
export const ACTIVE_HIGHLIGHT_CLASS = "rf-review-hl-active";

export const setHighlightsEffect = StateEffect.define<LineHighlight[]>();

function buildDecorations(
  view: { state: EditorView["state"] },
  highlights: LineHighlight[],
): DecorationSet {
  const doc = view.state.doc;
  const perLine = new Map<number, Set<string>>();
  for (const h of highlights) {
    const first = Math.max(1, Math.min(h.startLine, doc.lines));
    const last = Math.max(first, Math.min(h.endLine, doc.lines));
    for (let n = first; n <= last; n++) {
      const classes = perLine.get(n) ?? new Set<string>();
      classes.add(HIGHLIGHT_CLASS[h.kind]);
      if (h.active) classes.add(ACTIVE_HIGHLIGHT_CLASS);
      perLine.set(n, classes);
    }
  }
  const builder = new RangeSetBuilder<Decoration>();
  [...perLine.keys()]
    .sort((a, b) => a - b)
    .forEach((n) => {
      const from = doc.line(n).from;
      const classes = perLine.get(n);
      if (!classes) return;
      builder.add(
        from,
        from,
        Decoration.line({ class: [...classes].join(" ") }),
      );
    });
  return builder.finish();
}

const highlightField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, transaction) {
    let next = value.map(transaction.changes);
    for (const effect of transaction.effects) {
      if (effect.is(setHighlightsEffect)) {
        next = buildDecorations(transaction, effect.value);
      }
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

export type LineRange = { startLine: number; endLine: number };

/**
 * Decorations for review ranges plus a selection observer that reports the
 * selected 1-based line range. A caret without a selection reports null so
 * clicking around never creates an anchor implicitly.
 */
export function reviewSideExtension(
  onSelection: (range: LineRange | null) => void,
): Extension {
  return [
    highlightField,
    EditorView.updateListener.of((update) => {
      if (!update.selectionSet) return;
      const main = update.state.selection.main;
      if (main.empty) {
        onSelection(null);
        return;
      }
      const doc = update.state.doc;
      const startLine = doc.lineAt(main.from).number;
      const endPos = main.to > main.from ? main.to - 1 : main.to;
      const endLine = Math.max(startLine, doc.lineAt(endPos).number);
      onSelection({ startLine, endLine });
    }),
  ];
}

export function applyHighlights(
  view: EditorView,
  highlights: LineHighlight[],
): void {
  view.dispatch({ effects: setHighlightsEffect.of(highlights) });
}

/** Scroll a line into view without moving focus or the selection. */
export function revealLine(view: EditorView, line: number): void {
  const doc = view.state.doc;
  const clamped = Math.max(1, Math.min(line, doc.lines));
  view.dispatch({
    effects: EditorView.scrollIntoView(doc.line(clamped).from, {
      y: "center",
    }),
  });
}
