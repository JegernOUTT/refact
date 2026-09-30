#!/usr/bin/env python3
"""Print an advisory report of large, non-test Rust functions."""

from __future__ import annotations

import argparse
import bisect
import glob
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ENGINE_DIR = Path(__file__).resolve().parent.parent

TOKEN_PATTERN = re.compile(
    r"(?P<space>\s+)|(?P<line>//[^\n]*(?:\n|$))|(?P<block>/\*)|"
    r'(?P<raw>br(?P<byte_hashes>#+)?"|r(?P<hashes>#+)?")|'
    r'(?P<string>b?"(?:\\.|[^"\\])*")|'
    r"(?P<char>'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\r\n])')|"
    r"(?P<keyword>\b(?:fn|mod|cfg|test|pub|crate|unsafe|async)\b)|"
    r"(?P<punct>->|=>|[{}()\[\]#;<>-])|(?P<apostrophe>')|(?P<other>[^\s{}()\[\]#;<>-]+)",
    re.UNICODE,
)


@dataclass(frozen=True)
class Token:
    text: str
    line: int
    offset: int


@dataclass(frozen=True)
class Function:
    lines: int
    path: str
    line: int
    name: str


def tokenize(source: str) -> list[Token]:
    starts = [0]
    starts.extend(i + 1 for i, char in enumerate(source) if char == "\n")
    tokens: list[Token] = []
    skip_to = 0
    for match in TOKEN_PATTERN.finditer(source):
        if match.start() < skip_to:
            continue
        kind = match.lastgroup
        if kind == "space":
            continue
        if kind == "line":
            newline = source.find("\n", match.start())
            skip_to = len(source) if newline < 0 else newline + 1
            continue
        if kind == "block":
            depth = 1
            position = match.end()
            while depth:
                opening = source.find("/*", position)
                closing = source.find("*/", position)
                if closing < 0:
                    position = len(source)
                    break
                if opening >= 0 and opening < closing:
                    depth += 1
                    position = opening + 2
                else:
                    depth -= 1
                    position = closing + 2
            skip_to = position
            continue
        token_line = bisect.bisect_right(starts, match.start())
        if kind == "raw":
            hashes = match.groupdict().get("b_hashes") or match.groupdict().get("hashes") or ""
            terminator = '"' + hashes
            closing = source.find(terminator, match.end())
            skip_to = len(source) if closing < 0 else closing + len(terminator)
            tokens.append(Token("<literal>", token_line, match.start()))
            continue
        elif kind in ("string", "char"):
            tokens.append(Token("<literal>", token_line, match.start()))
        else:
            tokens.append(Token(match.group(), token_line, match.start()))
    return tokens


def delimiter_pairs(tokens: list[Token]) -> dict[int, int]:
    opening = {"{": "}", "[": "]", "(": ")"}
    closing = {value: key for key, value in opening.items()}
    stack: list[tuple[str, int]] = []
    pairs: dict[int, int] = {}
    for index, token in enumerate(tokens):
        if token.text in opening:
            stack.append((token.text, index))
        elif token.text in closing and stack and stack[-1][0] == closing[token.text]:
            _, start = stack.pop()
            pairs[start] = index
            pairs[index] = start
    return pairs


def function_body(tokens: list[Token], pairs: dict[int, int], start: int) -> int | None:
    angle_depth = 0
    index = start
    while index < len(tokens):
        text = tokens[index].text
        if text in ("(", "[") and index in pairs:
            index = pairs[index] + 1
            continue
        if text == "<":
            angle_depth += 1
        elif text == ">" and (index == 0 or tokens[index - 1].text not in ("-", "=")):
            angle_depth = max(0, angle_depth - 1)
        elif text == "{" and angle_depth == 0:
            return index if index in pairs else None
        elif text == ";" and angle_depth == 0:
            return None
        index += 1
    return None


def test_module_ranges(tokens: list[Token], pairs: dict[int, int]) -> list[tuple[int, int]]:
    ranges = []
    for index, token in enumerate(tokens):
        if token.text != "#" or index + 1 >= len(tokens) or tokens[index + 1].text != "[":
            continue
        attribute = index + 1
        if attribute not in pairs:
            continue
        end = pairs[attribute]
        if [item.text for item in tokens[attribute + 1 : end]] != ["cfg", "(", "test", ")"]:
            continue
        module = end + 1
        if module < len(tokens) and tokens[module].text == "pub":
            module += 1
            if module < len(tokens) and tokens[module].text == "(":
                module = pairs.get(module, module) + 1
        if module < len(tokens) and tokens[module].text == "mod":
            body = function_body(tokens, pairs, module + 1)
            if body is not None:
                ranges.append((body, pairs[body]))
    return ranges


def functions_in_source(source: str, path: str) -> list[Function]:
    tokens = tokenize(source)
    pairs = delimiter_pairs(tokens)
    excluded = test_module_ranges(tokens, pairs)
    functions = []
    for index, token in enumerate(tokens):
        if token.text != "fn" or any(start < index < end for start, end in excluded):
            continue
        name_offset = token.offset + 2
        while name_offset < len(source) and source[name_offset].isspace():
            name_offset += 1
        if source.startswith("r#", name_offset):
            name_offset += 2
        name_match = re.match(r"[A-Za-z_]\w*", source[name_offset:], re.UNICODE)
        if name_match is None:
            continue
        body = function_body(tokens, pairs, index + 1)
        if body is None:
            continue
        end = pairs[body]
        functions.append(Function(tokens[end].line - token.line + 1, path, token.line, name_match.group()))
    return functions


def is_test_file(path: Path) -> bool:
    return path.name == "tests.rs" or path.name.endswith("_tests.rs") or "tests" in path.parts


def scan_roots(roots: list[Path], base_dir: Path) -> tuple[int, list[Function]]:
    files: set[Path] = set()
    for root in roots:
        if not root.is_dir():
            raise FileNotFoundError(f"Rust source root does not exist: {root}")
        files.update(file for file in root.rglob("*.rs") if not is_test_file(file))
    functions = []
    for file_path in sorted(files):
        try:
            relative = file_path.relative_to(base_dir).as_posix()
        except ValueError:
            relative = file_path.as_posix()
        with file_path.open("r", encoding="utf-8", buffering=256 * 1024) as source_file:
            functions.extend(functions_in_source(source_file.read(), relative))
    return len(files), functions


def default_roots() -> list[Path]:
    crates = sorted(glob.glob(str(ENGINE_DIR / "crates" / "*" / "src")))
    return [ENGINE_DIR / "src", *(Path(root) for root in crates)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=(
            "List large non-test Rust functions. Excludes #[cfg(test)] modules, "
            "tests.rs and *_tests.rs files, and any tests/ directory."
        )
    )
    parser.add_argument("--root", action="append", type=Path, help="source root to scan (repeatable; replaces defaults)")
    parser.add_argument("--threshold", type=int, default=250, help="show every function over this line count (default: 250)")
    parser.add_argument("--top", type=int, default=15, help="also show the largest N functions (default: 15)")
    args = parser.parse_args(argv)
    if args.threshold < 0 or args.top < 0:
        parser.error("--threshold and --top must be non-negative")
    roots = [root.resolve() for root in args.root] if args.root else default_roots()
    base_dir = Path.cwd().resolve() if args.root else ENGINE_DIR
    try:
        file_count, functions = scan_roots(roots, base_dir)
    except OSError as error:
        parser.error(str(error))
    ordered = sorted(functions, key=lambda item: (-item.lines, item.path, item.line, item.name))
    selected = {id(function) for function in ordered[: args.top]}
    selected.update(id(function) for function in ordered if function.lines > args.threshold)
    print(f"Scanned {file_count} Rust files; threshold: {args.threshold} lines")
    for function in ordered:
        if id(function) in selected:
            print(f"{function.lines:>5}  {function.path}:{function.line}  {function.name}")
    over = sum(function.lines > args.threshold for function in functions)
    print(f"{over} functions over {args.threshold} lines (advisory)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
