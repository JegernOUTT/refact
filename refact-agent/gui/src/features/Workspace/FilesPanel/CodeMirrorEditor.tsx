import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { bracketMatching, indentOnInput } from "@codemirror/language";
import {
  Annotation,
  Compartment,
  EditorState,
  RangeSetBuilder,
  StateEffect,
  StateField,
  Transaction,
  type Extension,
} from "@codemirror/state";
import {
  highlightSelectionMatches,
  search,
  searchKeymap,
} from "@codemirror/search";
import {
  Decoration,
  type DecorationSet,
  drawSelection,
  EditorView,
  keymap,
  lineNumbers,
  rectangularSelection,
  type ViewUpdate,
  ViewPlugin,
  WidgetType,
} from "@codemirror/view";
import { useEffect, useRef } from "react";

import { useAppearance } from "../../../hooks";
import type { DiffChunk } from "../../../services/refact";
import styles from "./CodeMirrorEditor.module.css";
import { resolveLanguageSupport } from "./codeMirrorLanguages";
import { createEditorTheme } from "./codeMirrorTheme";

export type CodeMirrorEditorProps = {
  value: string;
  language: string | null;
  readOnly: boolean;
  ariaLabel: string;
  lineStart?: number;
  targetLine?: number;
  changedLines?: number[];
  changeRevision?: string;
  removedChunks?: DiffChunk[];
  onChange?: (value: string) => void;
  onSave?: () => void;
  onCancel?: () => void;
  onRequestEdit?: (offset: number) => void;
  onReady?: (view: EditorView) => void;
};

type LineDecorationConfig = {
  lineStart: number;
  targetLine?: number;
  changedLines: Set<number>;
};

const valueSync = Annotation.define<boolean>();
const MAX_GHOST_LINES_PER_CHUNK = 40;

type RemovedChunksConfig = {
  chunks: DiffChunk[];
  lineStart: number;
};

const setRemovedChunks = StateEffect.define<RemovedChunksConfig>();

class RemovedLineWidget extends WidgetType {
  constructor(readonly text: string) {
    super();
  }

  eq(other: RemovedLineWidget): boolean {
    return other.text === this.text;
  }

  toDOM(): HTMLElement {
    const element = document.createElement("div");
    element.className = styles.removedLine;
    element.setAttribute("data-live-removed", "true");
    element.setAttribute("aria-hidden", "true");
    element.textContent = this.text || " ";
    return element;
  }
}

function buildRemovedDecorations(
  state: EditorState,
  config: RemovedChunksConfig,
): DecorationSet {
  const byAnchor = new Map<number, string[]>();

  for (const chunk of config.chunks) {
    const removed = chunk.lines_remove.replace(/\n$/, "");
    if (!removed) continue;
    const documentLine = chunk.line1 - config.lineStart + 1;
    if (documentLine < 1 || documentLine > state.doc.lines) continue;
    const lines = removed.split("\n").slice(0, MAX_GHOST_LINES_PER_CHUNK);
    byAnchor.set(documentLine, [
      ...(byAnchor.get(documentLine) ?? []),
      ...lines,
    ]);
  }

  const builder = new RangeSetBuilder<Decoration>();
  const anchors = [...byAnchor.keys()].sort((left, right) => left - right);

  for (const documentLine of anchors) {
    const from = state.doc.line(documentLine).from;
    for (const text of byAnchor.get(documentLine) ?? []) {
      builder.add(
        from,
        from,
        Decoration.widget({
          widget: new RemovedLineWidget(text),
          block: true,
          side: -1,
        }),
      );
    }
  }

  return builder.finish();
}

const removedLinesField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update: (value, transaction) => {
    let next = value.map(transaction.changes);
    for (const effect of transaction.effects) {
      if (effect.is(setRemovedChunks)) {
        next = buildRemovedDecorations(transaction.state, effect.value);
      }
    }
    return next;
  },
  provide: (field) => EditorView.decorations.from(field),
});

function createLineDecorations(
  view: EditorView,
  config: LineDecorationConfig,
): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const decoratedLines = new Set<number>();

  for (const range of view.visibleRanges) {
    let line = view.state.doc.lineAt(range.from);
    while (line.from <= range.to) {
      if (!decoratedLines.has(line.number)) {
        const lineNumber = config.lineStart + line.number - 1;
        const attributes: Record<string, string> = {
          "data-line-number": String(lineNumber),
        };
        if (lineNumber === config.targetLine) {
          attributes["data-target-line"] = "true";
          attributes.id = "files-panel-target-line";
        }
        if (config.changedLines.has(lineNumber)) {
          attributes["data-live-change"] = "true";
        }
        builder.add(line.from, line.from, Decoration.line({ attributes }));
        decoratedLines.add(line.number);
      }
      if (line.to >= view.state.doc.length) break;
      line = view.state.doc.line(line.number + 1);
    }
  }

  return builder.finish();
}

function lineDecorationPlugin(config: LineDecorationConfig): Extension {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;

      constructor(view: EditorView) {
        this.decorations = createLineDecorations(view, config);
      }

      update(update: ViewUpdate) {
        this.decorations = createLineDecorations(update.view, config);
      }
    },
    { decorations: (plugin) => plugin.decorations },
  );
}

function editableExtension(readOnly: boolean): Extension {
  return [EditorState.readOnly.of(readOnly), EditorView.editable.of(!readOnly)];
}

export function CodeMirrorEditor({
  value,
  language,
  readOnly,
  ariaLabel,
  lineStart,
  targetLine,
  changedLines,
  changeRevision,
  removedChunks,
  onChange,
  onSave,
  onCancel,
  onRequestEdit,
  onReady,
}: CodeMirrorEditorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const readOnlyCompartment = useRef(new Compartment());
  const languageCompartment = useRef(new Compartment());
  const themeCompartment = useRef(new Compartment());
  const contentAttributesCompartment = useRef(new Compartment());
  const linePresentationCompartment = useRef(new Compartment());
  const initialValueRef = useRef(value);
  const lineConfigRef = useRef<LineDecorationConfig>({
    lineStart: lineStart ?? 1,
    targetLine,
    changedLines: new Set(changedLines ?? []),
  });
  const callbacksRef = useRef({ onChange, onSave, onCancel, onRequestEdit });
  const readOnlyRef = useRef(readOnly);
  const ariaLabelRef = useRef(ariaLabel);
  const onReadyRef = useRef(onReady);
  const { appearance } = useAppearance();
  const initialDarkRef = useRef(appearance === "dark");

  callbacksRef.current = { onChange, onSave, onCancel, onRequestEdit };
  readOnlyRef.current = readOnly;
  ariaLabelRef.current = ariaLabel;
  onReadyRef.current = onReady;
  lineConfigRef.current.lineStart = lineStart ?? 1;
  lineConfigRef.current.targetLine = targetLine;
  lineConfigRef.current.changedLines = new Set(changedLines ?? []);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const lineConfig = lineConfigRef.current;
    const state = EditorState.create({
      doc: initialValueRef.current,
      extensions: [
        readOnlyCompartment.current.of(editableExtension(readOnlyRef.current)),
        languageCompartment.current.of([]),
        themeCompartment.current.of(createEditorTheme(initialDarkRef.current)),
        contentAttributesCompartment.current.of(
          EditorView.contentAttributes.of({
            "aria-label": ariaLabelRef.current,
          }),
        ),
        linePresentationCompartment.current.of([
          lineDecorationPlugin(lineConfig),
          lineNumbers({
            formatNumber: (number) => String(lineConfig.lineStart + number - 1),
          }),
        ]),
        removedLinesField,
        history(),
        drawSelection(),
        rectangularSelection(),
        bracketMatching(),
        indentOnInput(),
        EditorState.allowMultipleSelections.of(true),
        search({ top: true }),
        highlightSelectionMatches(),
        EditorView.domEventHandlers({
          dblclick: (event, view) => {
            if (
              !readOnlyRef.current ||
              !callbacksRef.current.onRequestEdit ||
              !(event.target instanceof Node) ||
              !view.contentDOM.contains(event.target)
            ) {
              return false;
            }
            const position = view.posAtCoords({
              x: event.clientX,
              y: event.clientY,
            });
            callbacksRef.current.onRequestEdit(
              position ?? view.state.selection.main.head,
            );
            return false;
          },
        }),
        keymap.of([
          {
            key: "Mod-s",
            run: () => {
              if (!callbacksRef.current.onSave) return false;
              callbacksRef.current.onSave();
              return true;
            },
          },
          ...searchKeymap,
          {
            key: "Escape",
            run: () => {
              if (!callbacksRef.current.onCancel) return false;
              callbacksRef.current.onCancel();
              return true;
            },
          },
          ...defaultKeymap,
          ...historyKeymap,
        ]),
        EditorView.updateListener.of((update) => {
          if (
            update.docChanged &&
            !update.transactions.some(
              (transaction) => transaction.annotation(valueSync) === true,
            )
          ) {
            callbacksRef.current.onChange?.(update.state.doc.toString());
          }
        }),
      ],
    });
    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    onReadyRef.current?.(view);

    return () => {
      viewRef.current = null;
      view.destroy();
    };
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view || view.state.doc.toString() === value) return;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: value },
      annotations: [valueSync.of(true), Transaction.addToHistory.of(false)],
    });
  }, [value]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: readOnlyCompartment.current.reconfigure(
        editableExtension(readOnly),
      ),
    });
  }, [readOnly]);

  useEffect(() => {
    let stale = false;
    void resolveLanguageSupport(language).then((support) => {
      const view = viewRef.current;
      if (stale || !view) return;
      view.dispatch({
        effects: languageCompartment.current.reconfigure(support ?? []),
      });
    });
    return () => {
      stale = true;
    };
  }, [language]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: themeCompartment.current.reconfigure(
        createEditorTheme(appearance === "dark"),
      ),
    });
  }, [appearance]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: contentAttributesCompartment.current.reconfigure(
        EditorView.contentAttributes.of({ "aria-label": ariaLabel }),
      ),
    });
  }, [ariaLabel]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const config = lineConfigRef.current;
    view.dispatch({
      effects: linePresentationCompartment.current.reconfigure([
        lineDecorationPlugin(config),
        lineNumbers({
          formatNumber: (number) => String(config.lineStart + number - 1),
        }),
      ]),
    });
  }, [lineStart, targetLine, changedLines, changeRevision]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    view.dispatch({
      effects: setRemovedChunks.of({
        chunks: removedChunks ?? [],
        lineStart: lineStart ?? 1,
      }),
    });
  }, [changeRevision, lineStart, removedChunks]);

  useEffect(() => {
    const view = viewRef.current;
    if (targetLine === undefined || !view) return;
    const documentLine = targetLine - (lineStart ?? 1) + 1;
    if (documentLine < 1 || documentLine > view.state.doc.lines) return;
    const position = view.state.doc.line(documentLine).from;
    view.dispatch({
      effects: EditorView.scrollIntoView(position, { y: "center" }),
    });
  }, [lineStart, targetLine]);

  return <div ref={hostRef} className={styles.host} />;
}
