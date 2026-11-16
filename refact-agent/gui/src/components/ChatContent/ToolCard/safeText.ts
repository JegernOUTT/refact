/**
 * Display safety for engine- and script-provided strings: invisible
 * formatting characters (bidi controls, zero-width characters, BOM) are
 * replaced by a visible marker so text cannot reorder itself or hide a
 * difference, and long strings are cut with the full value kept separately.
 */

// Bidi embeddings/overrides/isolates, marks, zero-width and other invisible format characters.
const INVISIBLE =
  /[\u061C\u180E\u200B-\u200F\u202A-\u202E\u2060-\u2064\u2066-\u206F\uFEFF]/g;

function marker(char: string): string {
  const code = char.codePointAt(0) ?? 0;
  return `⟨U+${code.toString(16).toUpperCase().padStart(4, "0")}⟩`;
}

/** The text with every invisible format character shown as ⟨U+XXXX⟩. */
export function visibleText(text: string): string {
  return text.replace(INVISIBLE, marker);
}

/** Default cut for titles, labels, scopes and function names. */
export const SHORT_TEXT_MAX = 120;

/** Safe display text cut to `max` code points, and the full safe text. */
export function shortText(
  text: string,
  max = SHORT_TEXT_MAX,
): { shown: string; full: string; cut: boolean } {
  const full = visibleText(text);
  const chars = Array.from(full);
  if (chars.length <= max) return { shown: full, full, cut: false };
  return { shown: `${chars.slice(0, max - 1).join("")}…`, full, cut: true };
}
