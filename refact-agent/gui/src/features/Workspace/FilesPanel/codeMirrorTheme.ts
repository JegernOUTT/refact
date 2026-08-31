import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { tags } from "@lezer/highlight";

export function createEditorTheme(isDark: boolean): Extension[] {
  const palette = isDark
    ? {
        keyword: "#ff7b72",
        string: "#a5d6ff",
        number: "#79c0ff",
        comment: "#8b949e",
        function: "#d2a8ff",
        type: "#ffa657",
        variable: "#c9d1d9",
        tag: "#7ee787",
        attribute: "#79c0ff",
        heading: "#79c0ff",
        link: "#a5d6ff",
        invalid: "#f85149",
      }
    : {
        keyword: "#cf222e",
        string: "#0a3069",
        number: "#0550ae",
        comment: "#6e7781",
        function: "#8250df",
        type: "#953800",
        variable: "#24292f",
        tag: "#116329",
        attribute: "#0550ae",
        heading: "#0550ae",
        link: "#0a3069",
        invalid: "#f85149",
      };

  const theme = EditorView.theme({
    "&": {
      height: "100%",
      color: "var(--rf-color-fg)",
      background: "var(--rf-bg)",
      fontFamily: "var(--rf-font-mono)",
      fontSize: "var(--rf-text-2)",
      lineHeight: "var(--rf-line)",
    },
    "&.cm-focused": { outline: "none" },
    ".cm-scroller": {
      fontFamily: "inherit",
      fontSize: "inherit",
      lineHeight: "inherit",
      background: "inherit",
    },
    ".cm-content": { padding: "var(--rf-space-2) 0" },
    ".cm-gutters": {
      background: "var(--rf-bg)",
      border: "none",
      color: "var(--rf-color-faint)",
    },
    ".cm-lineNumbers .cm-gutterElement": {
      padding: "0 var(--rf-space-3)",
      textAlign: "right",
    },
    ".cm-activeLine, .cm-activeLineGutter": {
      backgroundColor: "color-mix(in srgb, currentColor 6%, transparent)",
    },
  });

  const highlightStyle = HighlightStyle.define([
    {
      tag: [
        tags.keyword,
        tags.controlKeyword,
        tags.operatorKeyword,
        tags.definitionKeyword,
        tags.moduleKeyword,
      ],
      color: palette.keyword,
    },
    { tag: [tags.string, tags.special(tags.string)], color: palette.string },
    { tag: [tags.number, tags.bool, tags.null], color: palette.number },
    {
      tag: [tags.comment, tags.lineComment, tags.blockComment, tags.docComment],
      color: palette.comment,
      fontStyle: "italic",
    },
    {
      tag: [tags.function(tags.variableName), tags.function(tags.propertyName)],
      color: palette.function,
    },
    {
      tag: [tags.typeName, tags.className, tags.namespace],
      color: palette.type,
    },
    {
      tag: [
        tags.definition(tags.variableName),
        tags.variableName,
        tags.propertyName,
      ],
      color: palette.variable,
    },
    { tag: tags.tagName, color: palette.tag },
    {
      tag: [tags.attributeName, tags.attributeValue],
      color: palette.attribute,
    },
    { tag: tags.heading, color: palette.heading, fontWeight: "bold" },
    { tag: tags.strong, fontWeight: "bold" },
    { tag: tags.emphasis, fontStyle: "italic" },
    {
      tag: [tags.link, tags.url],
      color: palette.link,
      textDecoration: "underline",
    },
    { tag: tags.meta, color: palette.comment },
    {
      tag: [tags.punctuation, tags.bracket],
      color: palette.variable,
    },
    { tag: tags.invalid, color: palette.invalid },
  ]);

  return [theme, syntaxHighlighting(highlightStyle)];
}
