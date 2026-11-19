//! One-line hints with a working alternative for errors this interpreter raises where
//! CPython would not.

/// (exception type, any of these message fragments, hint). Matched in order.
const ROWS: &[(&str, &[&str], &str)] = &[
    (
        "NotImplementedError",
        &["does not yet support the 'del' statement"],
        "del is not supported: use x = None to drop a value, d.pop(key, None) for a dict key, items.pop(i) or items[a:b] = [] for list items.",
    ),
    (
        "TypeError",
        &["'list' object is not an iterator"],
        "Generator expressions and map/filter/zip/enumerate/reversed return lists here: use next(iter(...), None) or index the list.",
    ),
    (
        "AttributeError",
        &[
            "module 're' has no attribute 'subn'",
            "'re.Pattern' object has no attribute 'subn'",
        ],
        "Use new = re.sub(p, r, s) and n = len(re.findall(p, s)).",
    ),
    (
        "TypeError",
        &["callable replacement is not yet supported"],
        "Build the result with re.finditer: out, last = [], 0; for m in re.finditer(p, s): out += [s[last:m.start()], f(m)]; last = m.end(); then ''.join(out) + s[last:].",
    ),
    (
        "AttributeError",
        &["'PosixPath' object has no attribute 'parents'"],
        "Use [Path(*p.parts[:i]) for i in range(len(p.parts) - 1, 0, -1)] or walk p.parent.",
    ),
    (
        "AttributeError",
        &[
            "'PosixPath' object has no attribute 'relative_to'",
            "'PosixPath' object has no attribute 'is_relative_to'",
        ],
        "Use os.path.relpath(p, base); a result starting with .. means outside base.",
    ),
    (
        "AttributeError",
        &[
            "'PosixPath' object has no attribute 'expanduser'",
            "'PosixPath' object has no attribute 'home'",
            "type object 'PosixPath' has no attribute 'home'",
        ],
        "Use os.path.expanduser('~/...').",
    ),
    (
        "AttributeError",
        &["'PosixPath' object has no attribute 'touch'"],
        "Use if not p.exists(): p.write_text('').",
    ),
    (
        "AttributeError",
        &["'PosixPath' object has no attribute 'match'"],
        "Use fnmatch.fnmatch(p.name, pattern).",
    ),
    (
        "AttributeError",
        &[
            "attribute 'readlink'",
            "attribute 'lstat'",
            "attribute 'symlink'",
            "attribute 'symlink_to'",
            "attribute 'samefile'",
        ],
        "Symlink calls are unavailable: os.path.islink(p) tests a link, os.path.realpath(p) resolves it.",
    ),
    (
        "NotImplementedError",
        &["cannot suspend in this synchronous context"],
        "File, tool and helper calls cannot run inside callbacks of sorted/min/max/map/filter: compute first, e.g. size = {p: p.stat().st_size for p in paths}, then sorted(paths, key=lambda p: size[p]).",
    ),
    (
        "NotImplementedError",
        &["iterating an unread _io.TextIOWrapper"],
        "Use for line in f:, f.readlines() or f.read().splitlines().",
    ),
    (
        "SyntaxError",
        &["Source is too deeply nested"],
        "Split long +/and/or chains, long elif ladders, chained calls and deep nesting into several statements.",
    ),
    (
        "NotImplementedError",
        &["class inheritance and metaclasses"],
        "Subclassing is not supported: use a plain class or @dataclass, and raise built-in exceptions with a message, e.g. raise ValueError('...').",
    ),
    (
        "NotImplementedError",
        &["yield expressions", "yield from expressions"],
        "Generators are not supported: build and return a list.",
    ),
    (
        "NotImplementedError",
        &["method decorators (classmethod/staticmethod/property)"],
        "Use plain methods or module-level functions; call a method instead of reading a @property.",
    ),
    (
        "NotImplementedError",
        &["async context managers", "async for loops"],
        "Use await on plain coroutines and asyncio.gather.",
    ),
    (
        "RuntimeError",
        &["exceptions can only be called with zero or one string argument"],
        "Exceptions take at most one str argument: raise KeyError(str(key)). This error cannot be caught.",
    ),
    (
        "TypeError",
        &["read_text() takes no", "write_text() takes exactly one argument"],
        "Files are always UTF-8 here: call p.read_text() or p.write_text(s) without encoding or errors; for undecodable data use p.read_bytes().decode('utf-8', errors='replace').",
    ),
    (
        "TypeError",
        &["'errors' argument is not yet supported"],
        "Read bytes and decode: Path(p).read_bytes().decode('utf-8', errors='replace').",
    ),
    (
        "ValueError",
        &["update modes ('+') are not yet supported"],
        "Open the file once to read and again to write.",
    ),
    (
        "ValueError",
        &["exclusive creation mode is not supported"],
        "Check os.path.exists(p) first, then open with 'w'.",
    ),
    (
        "TypeError",
        &["print() 'file' argument must be sys.stdout or sys.stderr"],
        "Build the text and call f.write(text).",
    ),
    (
        "AttributeError",
        &["'_io.TextIOWrapper' object has no attribute 'writelines'"],
        "Use f.write(''.join(lines)).",
    ),
    (
        "AttributeError",
        &["'_io.TextIOWrapper' object has no attribute 'write'"],
        "sys.stdout.write and sys.stderr.write are missing: use print(text, end='') or print(text, end='', file=sys.stderr).",
    ),
    (
        "AttributeError",
        &["module 'sys' has no attribute 'exit'"],
        "End the script normally (put the rest in a function and return), or raise SystemExit('reason') to stop with an error.",
    ),
    (
        "AttributeError",
        &["module 'asyncio' has no attribute"],
        "Only asyncio.gather, run and sleep exist: await asyncio.gather(*[f(x) for x in xs]) runs calls concurrently; the engine caps parallel nested calls.",
    ),
    (
        "NotImplementedError",
        &["gather() does not yet support keyword arguments"],
        "Catch errors per call: define async def safe(c) that returns await c or the exception it raised, then gather safe(...) calls.",
    ),
    (
        "AttributeError",
        &["type object 'str' has no attribute 'maketrans'"],
        "Translate with a dict: ''.join(table.get(c, c) for c in s).",
    ),
    (
        "AttributeError",
        &["module 'functools' has no attribute 'wraps'"],
        "Drop @functools.wraps; the wrapper works without it.",
    ),
    (
        "ImportError",
        &["cannot import name 'asdict' from 'dataclasses'"],
        "Build the dict explicitly: {'x': p.x, 'y': p.y}.",
    ),
    (
        "AttributeError",
        &["module 'collections' has no attribute 'OrderedDict'"],
        "Plain dict keeps insertion order.",
    ),
    (
        "TypeError",
        &["'<' not supported between instances of 'PosixPath'"],
        "Paths are not orderable: sorted(paths, key=str).",
    ),
    (
        "AttributeError",
        &["'function' object has no attribute 'glob'"],
        "glob is the file-search tool (await glob(pattern=...)); for local patterns use Path(dir).glob(...) or .rglob(...).",
    ),
];

/// Methods these types have as calls; reading one as a value is what fails.
const METHODS: &[(&str, &[&str])] = &[
    (
        "dict",
        &[
            "get",
            "items",
            "keys",
            "values",
            "pop",
            "setdefault",
            "update",
            "copy",
        ],
    ),
    (
        "list",
        &[
            "append", "extend", "insert", "remove", "pop", "index", "count", "sort", "copy",
        ],
    ),
    (
        "str",
        &[
            "lower",
            "upper",
            "strip",
            "lstrip",
            "rstrip",
            "split",
            "rsplit",
            "startswith",
            "endswith",
            "replace",
            "join",
            "find",
            "count",
            "title",
            "capitalize",
            "casefold",
            "splitlines",
        ],
    ),
    (
        "set",
        &[
            "add",
            "discard",
            "remove",
            "pop",
            "union",
            "intersection",
            "difference",
        ],
    ),
    ("tuple", &["index", "count"]),
];

const NOT_BUILTINS: &[&str] = &[
    "callable",
    "globals",
    "vars",
    "dir",
    "issubclass",
    "bytearray",
    "memoryview",
    "super",
    "delattr",
    "input",
];

/// The hint for an error, when there is one.
pub fn hint(exc_type: &str, message: &str) -> Option<String> {
    if let Some((_, _, hint)) = ROWS.iter().find(|(kind, needles, _)| {
        *kind == exc_type && needles.iter().any(|needle| message.contains(needle))
    }) {
        return Some((*hint).into());
    }
    match exc_type {
        "ModuleNotFoundError" => module_hint(quoted(message, "No module named ")?),
        "AttributeError" => method_hint(message),
        "NameError" => {
            let name = quoted(message, "name ")?;
            NOT_BUILTINS
                .contains(&name)
                .then(|| format!("{name}() does not exist in this runtime."))
        }
        _ => None,
    }
}

fn quoted<'a>(message: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = message.strip_prefix(prefix)?.strip_prefix('\'')?;
    rest.split('\'').next()
}

fn module_hint(module: &str) -> Option<String> {
    Some(
        match module.split('.').next()? {
            "subprocess" => "Run commands with the shell tool: await shell(command=..., description=...).",
            "urllib" | "http" | "requests" | "socket" => "Fetch URLs with the web tool.",
            "shutil" => "Copy with Path(dst).write_bytes(Path(src).read_bytes()), delete with os.remove or os.rmdir, move with os.rename, or use the shell tool.",
            "threading" | "concurrent" => "Use await asyncio.gather(...).",
            "glob" => "There is no glob module: for local patterns use Path(dir).glob(...) or .rglob(...), or the glob tool where this chat has it.",
            "collections" if module == "collections.abc" => "Import these names from typing.",
            _ => return Some(format!("This runtime has no '{module}' module; the python tool description lists the available ones.")),
        }
        .into(),
    )
}

fn method_hint(message: &str) -> Option<String> {
    let (owner, attribute) = message.split_once(" has no attribute ")?;
    let owner = owner.strip_prefix("type object ").unwrap_or(owner);
    let owner = owner
        .strip_suffix(" object")
        .unwrap_or(owner)
        .trim_matches('\'');
    let methods = METHODS.iter().find(|(kind, _)| *kind == owner)?.1;
    methods.contains(&attribute.trim_matches('\'')).then(|| {
        "Methods cannot be used as values here: pass key=lambda k: d[k] or lambda s: s.lower() instead of d.get or str.lower.".into()
    })
}

#[cfg(test)]
#[path = "hints_tests.rs"]
mod tests;
