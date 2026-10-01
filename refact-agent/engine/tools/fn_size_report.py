#!/usr/bin/env python3
"""Print an advisory report of large, non-test Rust functions."""

from __future__ import annotations

import argparse
import bisect
import glob
import re
import sys
from concurrent.futures import ProcessPoolExecutor
from dataclasses import dataclass
from pathlib import Path

ENGINE_DIR = Path(__file__).resolve().parent.parent
TOKEN_PATTERN = re.compile(
    r'(?P<line>//)|(?P<block>/\*)|(?P<raw>(?<![\w#])(?:br|r)(?P<hashes>#+)?")|'
    r'(?P<string>")|'
    r"(?P<char>b?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\r\n])')|"
    r'(?P<keyword>(?<![\w#])(?:fn|mod|cfg|test|any|all|pub)\b)|'
    r'(?P<punct>[{}()\[\]#;<>=])',
    re.UNICODE,
)
FN_NAME_PATTERN = re.compile(r"\s+(?:r#)?([A-Za-z_]\w*)", re.UNICODE)
MOD_DECL_PATTERN = re.compile(r"\s+([A-Za-z_]\w*)\s*;")


@dataclass(frozen=True)
class Function:
    lines: int
    path: str
    line: int
    name: str


def quoted_literal_end(source: str, start: int, quote: str) -> int:
    position = start + 1
    while position < len(source):
        if source[position] == "\\":
            position += 2
        elif source[position] == quote:
            return position + 1
        else:
            position += 1
    return len(source)


def tokenize(source: str) -> list[tuple[str, int]]:
    tokens: list[tuple[str, int]] = []
    skip_to = 0
    for match in TOKEN_PATTERN.finditer(source):
        start = match.start()
        if start < skip_to:
            continue
        kind = match.lastgroup
        end = match.end()
        if kind == "line":
            newline = source.find("\n", end)
            skip_to = len(source) if newline < 0 else newline + 1
            continue
        if kind == "block":
            depth = 1
            position = end
            while depth:
                opening = source.find("/*", position)
                closing = source.find("*/", position)
                if closing < 0:
                    skip_to = len(source)
                    break
                if opening >= 0 and opening < closing:
                    depth += 1
                    position = opening + 2
                else:
                    depth -= 1
                    position = closing + 2
                    if depth == 0:
                        skip_to = position
            continue
        if kind == "raw":
            hashes = match.group("hashes") or ""
            terminator = '"' + hashes
            closing = source.find(terminator, end)
            skip_to = len(source) if closing < 0 else closing + len(terminator)
            continue
        if kind == "string":
            skip_to = quoted_literal_end(source, start, '"')
            continue
        if kind == "char":
            continue
        tokens.append((match.group(), start))
    return tokens


def delimiter_pairs(tokens: list[tuple[str, int]]) -> dict[int, int]:
    opening = {"{": "}", "[": "]", "(": ")"}
    closing = {value: key for key, value in opening.items()}
    stack: list[tuple[str, int]] = []
    pairs: dict[int, int] = {}
    for index, (text, _) in enumerate(tokens):
        if text in opening:
            stack.append((text, index))
        elif text in closing and stack and stack[-1][0] == closing[text]:
            _, start = stack.pop()
            pairs[start] = index
            pairs[index] = start
    return pairs


def function_body(tokens: list[tuple[str, int]], pairs: dict[int, int], start: int) -> int | None:
    angle_depth = 0
    index = start
    while index < len(tokens):
        text = tokens[index][0]
        if text in ("(", "[") and index in pairs:
            index = pairs[index] + 1
            continue
        if text == "<" and index + 1 < len(tokens) and tokens[index + 1][0] not in ("=", "<"):
            angle_depth += 1
        elif text == ">" and (index == 0 or tokens[index - 1][0] not in ("-", "=")):
            angle_depth = max(0, angle_depth - 1)
        elif text == "=" and index + 1 < len(tokens) and tokens[index + 1][0] == ">":
            angle_depth = 0
        elif text == "{" and angle_depth == 0:
            return index if index in pairs else None
        elif text == ";" and angle_depth == 0:
            return None
        index += 1
    return None


def attribute_end(tokens: list[tuple[str, int]], pairs: dict[int, int], start: int) -> int | None:
    if start + 1 >= len(tokens) or tokens[start][0] != "#" or tokens[start + 1][0] != "[":
        return None
    return pairs.get(start + 1)


def is_cfg_test_attribute(items: list[tuple[str, int]]) -> bool:
    texts = [text for text, _ in items]
    return texts == ["cfg", "(", "test", ")"] or (
        len(texts) >= 6
        and texts[:3] in (["cfg", "(", "any"], ["cfg", "(", "all"])
        and texts[-1] == ")"
        and "test" in texts[3:-1]
    )


def test_module_declarations(
    source: str, tokens: list[tuple[str, int]], pairs: dict[int, int]
) -> tuple[list[tuple[int, int]], list[str]]:
    ranges: list[tuple[int, int]] = []
    out_of_line: list[str] = []
    for index, (text, _) in enumerate(tokens):
        if text != "#":
            continue
        end = attribute_end(tokens, pairs, index)
        if end is None or not is_cfg_test_attribute(tokens[index + 2 : end]):
            continue
        module = end + 1
        while module < len(tokens):
            next_end = attribute_end(tokens, pairs, module)
            if next_end is None:
                break
            module = next_end + 1
        if module < len(tokens) and tokens[module][0] == "pub":
            module += 1
            if module < len(tokens) and tokens[module][0] == "(":
                module = pairs.get(module, module) + 1
        if module >= len(tokens) or tokens[module][0] != "mod":
            continue
        declaration = MOD_DECL_PATTERN.match(source, tokens[module][1] + 3)
        body = function_body(tokens, pairs, module + 1)
        if body is not None:
            ranges.append((body, pairs[body]))
        elif declaration is not None:
            out_of_line.append(declaration.group(1))
    return ranges, out_of_line


def has_test_attribute(tokens: list[tuple[str, int]], pairs: dict[int, int], fn_index: int) -> bool:
    cursor = fn_index
    while cursor > 0 and tokens[cursor - 1][0] in ("async", "unsafe", "const", "extern"):
        cursor -= 1
    found = False
    while cursor > 0 and tokens[cursor - 1][0] == "]":
        close = cursor - 1
        opening = pairs.get(close)
        if opening is None or opening == 0 or tokens[opening - 1][0] != "#":
            break
        contents = tokens[opening + 1 : close]
        if contents and contents[-1][0] == "test":
            found = True
        cursor = opening - 1
    return found


def analyze_source(source: str, path: str) -> tuple[list[Function], list[str]]:
    tokens = tokenize(source)
    pairs = delimiter_pairs(tokens)
    excluded, out_of_line = test_module_declarations(source, tokens, pairs)
    excluded.sort()
    candidates: list[tuple[int, int, str]] = []
    range_index = 0
    for index, (text, offset) in enumerate(tokens):
        while range_index < len(excluded) and excluded[range_index][1] <= index:
            range_index += 1
        in_test_module = range_index < len(excluded) and excluded[range_index][0] < index < excluded[range_index][1]
        if text != "fn" or has_test_attribute(tokens, pairs, index) or in_test_module:
            continue
        name_match = FN_NAME_PATTERN.match(source, offset + 2)
        if name_match is None:
            continue
        body = function_body(tokens, pairs, index + 1)
        if body is None:
            continue
        end = pairs[body]
        candidates.append((end, offset, name_match.group(1)))
    if not candidates:
        return [], out_of_line
    line_starts = [0]
    cursor = 0
    while (cursor := source.find("\n", cursor)) >= 0:
        line_starts.append(cursor + 1)
        cursor += 1
    result: list[Function] = []
    for close_index, fn_token_offset, name in candidates:
        close_offset = tokens[close_index][1]
        fn_line = bisect.bisect_right(line_starts, fn_token_offset)
        close_line = bisect.bisect_right(line_starts, close_offset)
        result.append(Function(close_line - fn_line + 1, path, fn_line, name))
    return result, out_of_line


def functions_in_source(source: str, path: str) -> list[Function]:
    return analyze_source(source, path)[0]


def is_test_file(path: Path) -> bool:
    return path.name == "tests.rs" or path.name.endswith("_tests.rs") or "tests" in path.parts


def process_source_file(arguments: tuple[str, str]) -> tuple[str, list[Function], list[str]]:
    path_text, base_text = arguments
    file_path = Path(path_text)
    base_dir = Path(base_text)
    try:
        relative = file_path.relative_to(base_dir).as_posix()
    except ValueError:
        relative = file_path.as_posix()
    with file_path.open("r", encoding="utf-8", buffering=256 * 1024) as source_file:
        found, out_of_line = analyze_source(source_file.read(), relative)
    return relative, found, out_of_line


def scan_roots(roots: list[Path], base_dir: Path) -> tuple[int, list[Function]]:
    files: set[Path] = set()
    for root in roots:
        if not root.is_dir():
            raise FileNotFoundError(f"Rust source root does not exist: {root}")
        files.update(file for file in root.rglob("*.rs") if not is_test_file(file.relative_to(root)))
    functions: list[Function] = []
    excluded_files: set[Path] = set()
    paths = sorted(files)
    arguments = [(str(file_path), str(base_dir)) for file_path in paths]

    try:
        with ProcessPoolExecutor(max_workers=min(8, len(paths))) as executor:
            for relative, found, out_of_line in executor.map(process_source_file, arguments, chunksize=16):
                functions.extend(found)
                file_path = base_dir / relative
                for name in out_of_line:
                    candidates = (
                        file_path.parent / f"{name}.rs",
                        file_path.parent / name / "mod.rs",
                        file_path.parent / file_path.stem / f"{name}.rs",
                        file_path.parent / file_path.stem / name / "mod.rs",
                    )
                    excluded_files.update(candidate.resolve() for candidate in candidates if candidate.exists())
    except (OSError, RuntimeError):
        functions.clear()
        excluded_files.clear()
        for file_path, argument in zip(paths, arguments):
            relative, found, out_of_line = process_source_file(argument)
            functions.extend(found)
            for name in out_of_line:
                candidates = (
                    file_path.parent / f"{name}.rs",
                    file_path.parent / name / "mod.rs",
                    file_path.parent / file_path.stem / f"{name}.rs",
                    file_path.parent / file_path.stem / name / "mod.rs",
                )
                excluded_files.update(candidate.resolve() for candidate in candidates if candidate.exists())
    excluded_files.intersection_update(files)
    excluded_relative = set()
    for file_path in excluded_files:
        try:
            excluded_relative.add(file_path.relative_to(base_dir).as_posix())
        except ValueError:
            excluded_relative.add(file_path.as_posix())
    functions = [function for function in functions if function.path not in excluded_relative]
    return len(files) - len(excluded_files), functions


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
