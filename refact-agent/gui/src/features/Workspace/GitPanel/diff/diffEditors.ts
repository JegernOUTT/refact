import {
  getChunks,
  goToNextChunk,
  goToPreviousChunk,
  MergeView,
  unifiedMergeView,
  type Chunk,
} from "@codemirror/merge";
import type { LanguageSupport } from "@codemirror/language";
import {
  Compartment,
  EditorState,
  RangeSetBuilder,
  StateField,
  type Extension,
  type Text,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  drawSelection,
  EditorView,
  GutterMarker,
  gutterLineClass,
  highlightSpecialChars,
  lineNumbers,
} from "@codemirror/view";
import type { RangeSet } from "@codemirror/state";

import { createEditorTheme } from "../../FilesPanel/codeMirrorTheme";

export type DiffMode = "side-by-side" | "unified";

/** JetBrains-style chunk classification. */
export type DiffChunkKind = "inserted" | "deleted" | "modified";

export const DIFF_LINE_CLASS: Record<DiffChunkKind, string> = {
  inserted: "rf-diff-inserted",
  deleted: "rf-diff-deleted",
  modified: "rf-diff-modified",
};

const DIFF_GUTTER_CLASS: Record<DiffChunkKind, string> = {
  inserted: "rf-diff-gutter-inserted",
  deleted: "rf-diff-gutter-deleted",
  modified: "rf-diff-gutter-modified",
};

export function chunkKind(chunk: Chunk): DiffChunkKind {
  if (chunk.fromA === chunk.toA) return "inserted";
  if (chunk.fromB === chunk.toB) return "deleted";
  return "modified";
}

class DiffGutterClass extends GutterMarker {
  constructor(readonly elementClass: string) {
    super();
  }
  eq(other: GutterMarker): boolean {
    return (
      other instanceof DiffGutterClass &&
      other.elementClass === this.elementClass
    );
  }
}

const GUTTER_MARKERS: Record<DiffChunkKind, DiffGutterClass> = {
  inserted: new DiffGutterClass(DIFF_GUTTER_CLASS.inserted),
  deleted: new DiffGutterClass(DIFF_GUTTER_CLASS.deleted),
  modified: new DiffGutterClass(DIFF_GUTTER_CLASS.modified),
};

const LINE_DECORATIONS: Record<DiffChunkKind, Decoration> = {
  inserted: Decoration.line({ class: DIFF_LINE_CLASS.inserted }),
  deleted: Decoration.line({ class: DIFF_LINE_CLASS.deleted }),
  modified: Decoration.line({ class: DIFF_LINE_CLASS.modified }),
};

type DiffClassValue = {
  chunks: readonly Chunk[] | null;
  lines: DecorationSet;
  gutter: RangeSet<GutterMarker>;
};

function buildDiffClasses(state: EditorState): DiffClassValue {
  const info = getChunks(state);
  const lines = new RangeSetBuilder<Decoration>();
  const gutter = new RangeSetBuilder<GutterMarker>();
  if (info) {
    const isA = info.side === "a";
    const doc = state.doc;
    for (const chunk of info.chunks) {
      const from = isA ? chunk.fromA : chunk.fromB;
      const to = isA ? chunk.toA : chunk.toB;
      if (from === to) continue;
      const kind = chunkKind(chunk);
      const first = doc.lineAt(Math.min(from, doc.length)).number;
      const last = doc.lineAt(
        Math.min(Math.max(from, to - 1), doc.length),
      ).number;
      for (let number = first; number <= last; number++) {
        const lineStart = doc.line(number).from;
        lines.add(lineStart, lineStart, LINE_DECORATIONS[kind]);
        gutter.add(lineStart, lineStart, GUTTER_MARKERS[kind]);
      }
    }
  }
  return {
    chunks: info?.chunks ?? null,
    lines: lines.finish(),
    gutter: gutter.finish(),
  };
}

/**
 * Tags every changed line with its JetBrains kind (inserted / deleted /
 * modified) so the palette tokens can tint whole lines and gutters; the
 * stock merge classes do not distinguish modified from inserted lines.
 */
const diffClassesField = StateField.define<DiffClassValue>({
  create: buildDiffClasses,
  update(value, transaction) {
    const chunks = getChunks(transaction.state)?.chunks ?? null;
    if (chunks === value.chunks && !transaction.docChanged) return value;
    return buildDiffClasses(transaction.state);
  },
  provide: (field) => [
    EditorView.decorations.from(field, (value) => value.lines),
    gutterLineClass.from(field, (value) => value.gutter),
  ],
});

export type DiffEditorSide = "a" | "b";

export type DiffEditorsConfig = {
  parent: HTMLElement;
  mode: DiffMode;
  left: string;
  right: string;
  collapseUnchanged: boolean;
  isDark: boolean;
  language: LanguageSupport | null;
  leftLabel: string;
  rightLabel: string;
  onFocusSide?: (side: DiffEditorSide) => void;
};

export type DiffEditorsController = {
  readonly mode: DiffMode;
  chunkCount: () => number;
  next: () => boolean;
  previous: () => boolean;
  /** 1-based line in the right document at the active cursor. */
  rightLineAtCursor: () => number;
  setDark: (isDark: boolean) => void;
  setLanguage: (language: LanguageSupport | null) => void;
  focus: () => void;
  scrollElement: () => HTMLElement;
  destroy: () => void;
};

const COLLAPSE_CONFIG = { margin: 3, minSize: 4 };

function sideExtensions(
  label: string,
  side: DiffEditorSide,
  theme: Compartment,
  language: Compartment,
  config: DiffEditorsConfig,
): Extension[] {
  return [
    EditorState.readOnly.of(true),
    lineNumbers(),
    highlightSpecialChars(),
    drawSelection(),
    theme.of(createEditorTheme(config.isDark)),
    language.of(config.language ?? []),
    EditorView.contentAttributes.of({ "aria-label": label }),
    EditorView.domEventHandlers({
      focus: () => {
        config.onFocusSide?.(side);
        return false;
      },
    }),
    diffClassesField,
  ];
}

/** Map a position in document A to the matching position in document B. */
function mapAToB(pos: number, chunks: readonly Chunk[]): number {
  let startA = 0;
  let startB = 0;
  for (const chunk of chunks) {
    if (chunk.fromA >= pos) break;
    if (chunk.toA > pos) return chunk.fromB;
    startA = chunk.toA;
    startB = chunk.toB;
  }
  return startB + (pos - startA);
}

function lineNumberAt(doc: Text, pos: number): number {
  return doc.lineAt(Math.max(0, Math.min(pos, doc.length))).number;
}

export function createDiffEditors(
  config: DiffEditorsConfig,
): DiffEditorsController {
  const themeA = new Compartment();
  const themeB = new Compartment();
  const languageA = new Compartment();
  const languageB = new Compartment();
  let activeSide: DiffEditorSide = "b";
  const trackedConfig: DiffEditorsConfig = {
    ...config,
    onFocusSide: (side) => {
      activeSide = side;
      config.onFocusSide?.(side);
    },
  };

  if (config.mode === "unified") {
    const view = new EditorView({
      parent: config.parent,
      state: EditorState.create({
        doc: config.right,
        extensions: [
          sideExtensions(
            config.rightLabel,
            "b",
            themeB,
            languageB,
            trackedConfig,
          ),
          unifiedMergeView({
            original: config.left,
            mergeControls: false,
            gutter: false,
            highlightChanges: true,
            syntaxHighlightDeletions: true,
            collapseUnchanged: config.collapseUnchanged
              ? COLLAPSE_CONFIG
              : undefined,
          }),
        ],
      }),
    });
    return {
      mode: "unified",
      chunkCount: () => getChunks(view.state)?.chunks.length ?? 0,
      next: () => goToNextChunk(view),
      previous: () => goToPreviousChunk(view),
      rightLineAtCursor: () =>
        lineNumberAt(view.state.doc, view.state.selection.main.head),
      setDark: (isDark) =>
        view.dispatch({
          effects: themeB.reconfigure(createEditorTheme(isDark)),
        }),
      setLanguage: (language) =>
        view.dispatch({ effects: languageB.reconfigure(language ?? []) }),
      focus: () => view.focus(),
      scrollElement: () => view.scrollDOM,
      destroy: () => view.destroy(),
    };
  }

  const merge = new MergeView({
    parent: config.parent,
    a: {
      doc: config.left,
      extensions: sideExtensions(
        config.leftLabel,
        "a",
        themeA,
        languageA,
        trackedConfig,
      ),
    },
    b: {
      doc: config.right,
      extensions: sideExtensions(
        config.rightLabel,
        "b",
        themeB,
        languageB,
        trackedConfig,
      ),
    },
    gutter: false,
    highlightChanges: true,
    collapseUnchanged: config.collapseUnchanged ? COLLAPSE_CONFIG : undefined,
  });
  const activeView = () => (activeSide === "a" ? merge.a : merge.b);

  return {
    mode: "side-by-side",
    chunkCount: () => merge.chunks.length,
    next: () => goToNextChunk(activeView()),
    previous: () => goToPreviousChunk(activeView()),
    rightLineAtCursor: () => {
      if (activeSide === "b") {
        return lineNumberAt(
          merge.b.state.doc,
          merge.b.state.selection.main.head,
        );
      }
      const pos = mapAToB(merge.a.state.selection.main.head, merge.chunks);
      return lineNumberAt(merge.b.state.doc, pos);
    },
    setDark: (isDark) => {
      merge.a.dispatch({
        effects: themeA.reconfigure(createEditorTheme(isDark)),
      });
      merge.b.dispatch({
        effects: themeB.reconfigure(createEditorTheme(isDark)),
      });
    },
    setLanguage: (language) => {
      merge.a.dispatch({ effects: languageA.reconfigure(language ?? []) });
      merge.b.dispatch({ effects: languageB.reconfigure(language ?? []) });
    },
    focus: () => activeView().focus(),
    scrollElement: () => merge.dom,
    destroy: () => merge.destroy(),
  };
}
