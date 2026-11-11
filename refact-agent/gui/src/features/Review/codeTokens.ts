export type CodeTokenKind = "" | "k" | "s" | "c" | "n";
export type CodeToken = [kind: CodeTokenKind, text: string];

const HASH_COMMENTS = new Set(
  "py rb sh bash zsh yaml yml toml r pl cfg ini conf dockerfile makefile".split(
    " ",
  ),
);
const DASH_COMMENTS = new Set(["sql", "lua", "hs"]);
const KEYWORDS = new Set(
  (
    "and as async await break case catch class const continue def default defer del do elif else " +
    "enum except export extends final finally fn for from func function if impl implements import " +
    "in interface is let match mod mut new not or pass pub raise return self static struct super " +
    "switch this throw trait try type use var while with yield None True False null true false nil " +
    "undefined void"
  ).split(" "),
);
const MAX_TOKENIZED_LINE = 400;

/** The line-comment marker of the language a path suggests. */
export function lineCommentOf(path: string): string {
  const name = path.split("/").at(-1)?.toLowerCase() ?? "";
  const extension = name.includes(".") ? name.split(".").at(-1) ?? "" : name;
  if (HASH_COMMENTS.has(extension)) return "#";
  return DASH_COMMENTS.has(extension) ? "--" : "//";
}

/**
 * A light, language-agnostic tint for one line: comments, strings, numbers
 * and common keywords. It never changes the text, only how spans are split.
 */
export function codeTokens(line: string, comment: string): CodeToken[] {
  if (line.length > MAX_TOKENIZED_LINE) return [["", line]];
  if (comment === "//" && /^\s*(\/\*|\*\/|\*(\s|$))/.test(line))
    return [["c", line]];
  const tokens: CodeToken[] = [];
  const push = (kind: CodeTokenKind, text: string) => {
    const last = tokens.at(-1);
    if (last && last[0] === kind) last[1] += text;
    else tokens.push([kind, text]);
  };
  let at = 0;
  while (at < line.length) {
    const rest = line.slice(at);
    if (rest.startsWith(comment)) {
      push("c", rest);
      break;
    }
    const found =
      /^"(?:\\.|[^"\\])*"/.exec(rest) ??
      /^`(?:\\.|[^`\\])*`/.exec(rest) ??
      /^'(?:\\.|[^'\\])'/.exec(rest) ??
      (comment === "#" ? /^'(?:\\.|[^'\\])*'/.exec(rest) : null);
    if (found) {
      push("s", found[0]);
      at += found[0].length;
      continue;
    }
    const word = /^[A-Za-z_$][\w$]*/.exec(rest);
    if (word) {
      push(KEYWORDS.has(word[0]) ? "k" : "", word[0]);
      at += word[0].length;
      continue;
    }
    const number = /^\d[\d_]*(?:\.\d+)?(?:[eE][+-]?\d+)?\w*/.exec(rest);
    if (number) {
      push("n", number[0]);
      at += number[0].length;
      continue;
    }
    push("", line[at]);
    at += 1;
  }
  return tokens;
}
