use std::collections::HashMap;

use crate::dry::DRY_DUPLICATION_THRESHOLD;
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

const MIN_CLONE_TOKENS: usize = 50;
const MAX_CLONE_PAIRS: usize = 200;
const MAX_HOT_BUCKET_DENSE_OCCURRENCES: usize = 64;
const BASE: u64 = 1_000_003;
const MODULUS: u64 = 9_223_372_036_854_775_783;
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub kind_hash: u64,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClonePair {
    pub line_a: usize,
    pub a_end_line: usize,
    pub line_b: usize,
    pub b_end_line: usize,
    pub token_len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossFileClonePair {
    pub file_a: String,
    pub line_a: usize,
    pub a_start_line: usize,
    pub a_end_line: usize,
    pub file_b: String,
    pub line_b: usize,
    pub b_start_line: usize,
    pub b_end_line: usize,
    pub token_len: usize,
}

#[derive(Debug, Clone)]
struct CrossFileCloneMatch {
    file_a: usize,
    file_b: usize,
    start_a: usize,
    start_b: usize,
    token_len: usize,
}

#[derive(Debug, Clone)]
struct CloneMatch {
    start_a: usize,
    start_b: usize,
    token_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowOccurrence {
    file_idx: usize,
    start: usize,
}

pub fn tokenize(lang: &str, text: &str) -> Vec<Token> {
    let Some(tree) = refact_codegraph_parsers::parse_tree(lang, text) else {
        return Vec::new();
    };

    let mut tokens = Vec::new();
    collect_tokens(tree.root_node(), text.as_bytes(), &mut tokens);
    tokens
}

fn collect_tokens(node: Node<'_>, bytes: &[u8], out: &mut Vec<Token>) {
    if node.is_named() && node.named_child_count() == 0 {
        let kind_hash = normalized_token_hash(node, bytes);
        out.push(Token {
            kind_hash,
            line: node.start_position().row + 1,
        });
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_tokens(child, bytes, out);
    }
}

fn normalized_token_hash(node: Node<'_>, bytes: &[u8]) -> u64 {
    let kind = node.kind();
    if is_identifier_kind(kind) {
        return fnv1a64(b"identifier");
    }
    if is_literal_kind(kind) {
        return fnv1a64(b"literal");
    }

    let text = node.utf8_text(bytes).unwrap_or(kind);
    fnv1a64(text.as_bytes())
}

fn is_identifier_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier" | "simple_identifier" | "field_identifier"
    ) || kind.ends_with("_identifier")
        || kind.ends_with("_name")
}

fn is_literal_kind(kind: &str) -> bool {
    matches!(
        kind,
        "literal"
            | "string"
            | "string_literal"
            | "raw_string_literal"
            | "char_literal"
            | "character_literal"
            | "integer"
            | "integer_literal"
            | "float"
            | "float_literal"
            | "number"
            | "number_literal"
            | "true"
            | "false"
            | "boolean"
            | "boolean_literal"
            | "null"
            | "nil"
            | "none"
    ) || kind.ends_with("_literal")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub fn detect_clones(lang: &str, text: &str) -> Vec<ClonePair> {
    let tokens = tokenize(lang, text);
    detect_clone_matches(&tokens)
        .into_iter()
        .map(|m| ClonePair {
            line_a: tokens[m.start_a].line,
            a_end_line: tokens[m.start_a + m.token_len - 1].line,
            line_b: tokens[m.start_b].line,
            b_end_line: tokens[m.start_b + m.token_len - 1].line,
            token_len: m.token_len,
        })
        .collect()
}

pub fn detect_cross_file_clones(files: &[(String, String, String)]) -> Vec<CrossFileClonePair> {
    let tokenized: Vec<(&str, Vec<Token>)> = files
        .iter()
        .map(|(path, lang, text)| (path.as_str(), tokenize(lang, text)))
        .collect();

    let mut matches = detect_cross_file_clone_matches(&tokenized);
    matches.truncate(MAX_CLONE_PAIRS);
    clone_pairs_from_matches(&tokenized, &matches)
}

fn clone_pairs_from_matches(
    tokenized: &[(&str, Vec<Token>)],
    matches: &[CrossFileCloneMatch],
) -> Vec<CrossFileClonePair> {
    matches
        .iter()
        .map(|m| {
            let (path_a, tokens_a) = &tokenized[m.file_a];
            let (path_b, tokens_b) = &tokenized[m.file_b];
            let a_start_line = tokens_a[m.start_a].line;
            let b_start_line = tokens_b[m.start_b].line;

            CrossFileClonePair {
                file_a: path_a.to_string(),
                line_a: a_start_line,
                a_start_line,
                a_end_line: tokens_a[m.start_a + m.token_len - 1].line,
                file_b: path_b.to_string(),
                line_b: b_start_line,
                b_start_line,
                b_end_line: tokens_b[m.start_b + m.token_len - 1].line,
                token_len: m.token_len,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct CrossFileDuplication {
    pub clones: Vec<CrossFileClonePair>,
    pub duplication_pct: f64,
    pub duplicated_tokens_by_path: HashMap<String, usize>,
    pub tokens_by_path: HashMap<String, usize>,
}

pub fn cross_file_analysis(files: &[(String, String, String)]) -> CrossFileDuplication {
    let tokenized: Vec<(&str, Vec<Token>)> = files
        .iter()
        .map(|(path, lang, text)| (path.as_str(), tokenize(lang, text)))
        .collect();

    let matches = detect_cross_file_clone_matches(&tokenized);
    let clones =
        clone_pairs_from_matches(&tokenized, &matches[..matches.len().min(MAX_CLONE_PAIRS)]);

    let mut duplicated: Vec<Vec<bool>> = tokenized
        .iter()
        .map(|(_, tokens)| vec![false; tokens.len()])
        .collect();
    for m in &matches {
        for idx in m.start_a..(m.start_a + m.token_len).min(duplicated[m.file_a].len()) {
            duplicated[m.file_a][idx] = true;
        }
        for idx in m.start_b..(m.start_b + m.token_len).min(duplicated[m.file_b].len()) {
            duplicated[m.file_b][idx] = true;
        }
    }

    let total_tokens: usize = tokenized.iter().map(|(_, tokens)| tokens.len()).sum();
    let duplicated_count: usize = duplicated
        .iter()
        .map(|file| file.iter().filter(|is_dup| **is_dup).count())
        .sum();
    let duplication_pct = if total_tokens == 0 {
        0.0
    } else {
        duplicated_count as f64 / total_tokens as f64
    };

    let mut duplicated_tokens_by_path = HashMap::new();
    let mut tokens_by_path = HashMap::new();
    for ((path, tokens), duplicated) in tokenized.iter().zip(duplicated) {
        let duplicated_count = duplicated.into_iter().filter(|is_dup| *is_dup).count();
        duplicated_tokens_by_path
            .entry((*path).to_string())
            .and_modify(|count| *count += duplicated_count)
            .or_insert(duplicated_count);
        tokens_by_path
            .entry((*path).to_string())
            .and_modify(|count| *count += tokens.len())
            .or_insert(tokens.len());
    }

    CrossFileDuplication {
        clones,
        duplication_pct,
        duplicated_tokens_by_path,
        tokens_by_path,
    }
}

fn detect_cross_file_clone_matches(tokenized: &[(&str, Vec<Token>)]) -> Vec<CrossFileCloneMatch> {
    let mut buckets: HashMap<u64, Vec<WindowOccurrence>> = HashMap::new();
    for (file_idx, (_, tokens)) in tokenized.iter().enumerate() {
        if tokens.len() < MIN_CLONE_TOKENS {
            continue;
        }

        for (start, hash) in rolling_window_hashes(tokens, MIN_CLONE_TOKENS)
            .into_iter()
            .enumerate()
        {
            buckets
                .entry(hash)
                .or_default()
                .push(WindowOccurrence { file_idx, start });
        }
    }

    let mut matches = Vec::new();
    let mut covered_pairs: HashMap<(usize, usize), Vec<((usize, usize), (usize, usize))>> =
        HashMap::new();
    let mut hashes: Vec<u64> = buckets.keys().copied().collect();
    hashes.sort_unstable();

    for hash in hashes {
        let Some(mut occurrences) = buckets.remove(&hash) else {
            continue;
        };
        if !bucket_has_cross_file_occurrences(&occurrences, tokenized) {
            continue;
        }

        occurrences.sort_unstable_by_key(|occurrence| (occurrence.file_idx, occurrence.start));
        if occurrences.len() > MAX_HOT_BUCKET_DENSE_OCCURRENCES {
            process_hot_cross_file_bucket(
                &occurrences,
                tokenized,
                &mut matches,
                &mut covered_pairs,
            );
        } else {
            process_cross_file_bucket_pairs(
                &occurrences,
                tokenized,
                &mut matches,
                &mut covered_pairs,
            );
        }
    }

    matches.sort_by_key(|m| (m.file_a, m.file_b, m.start_a, m.start_b, m.token_len));
    matches
}

fn bucket_has_cross_file_occurrences(
    occurrences: &[WindowOccurrence],
    tokenized: &[(&str, Vec<Token>)],
) -> bool {
    let mut first_path: Option<&str> = None;
    for occurrence in occurrences {
        let path = tokenized[occurrence.file_idx].0;
        let Some(first_path) = first_path else {
            first_path = Some(path);
            continue;
        };
        if first_path != path {
            return true;
        }
    }
    false
}

fn process_cross_file_bucket_pairs(
    occurrences: &[WindowOccurrence],
    tokenized: &[(&str, Vec<Token>)],
    matches: &mut Vec<CrossFileCloneMatch>,
    covered_pairs: &mut HashMap<(usize, usize), Vec<((usize, usize), (usize, usize))>>,
) {
    for i in 0..occurrences.len() {
        for j in (i + 1)..occurrences.len() {
            maybe_record_cross_file_match(
                occurrences[i],
                occurrences[j],
                tokenized,
                matches,
                covered_pairs,
            );
        }
    }
}

fn process_hot_cross_file_bucket(
    occurrences: &[WindowOccurrence],
    tokenized: &[(&str, Vec<Token>)],
    matches: &mut Vec<CrossFileCloneMatch>,
    covered_pairs: &mut HashMap<(usize, usize), Vec<((usize, usize), (usize, usize))>>,
) {
    let dense_len = occurrences.len().min(MAX_HOT_BUCKET_DENSE_OCCURRENCES);
    process_cross_file_bucket_pairs(&occurrences[..dense_len], tokenized, matches, covered_pairs);

    let representatives = distinct_path_representatives(occurrences, tokenized);
    // Dense pair expansion is capped for hot buckets; representative sweep keeps later files visible
    // without materializing all combinations.
    for occurrence in &occurrences[dense_len..] {
        let Some(representative) = representatives.iter().copied().find(|representative| {
            tokenized[representative.file_idx].0 != tokenized[occurrence.file_idx].0
        }) else {
            continue;
        };
        maybe_record_cross_file_match(
            representative,
            *occurrence,
            tokenized,
            matches,
            covered_pairs,
        );
    }
}

fn distinct_path_representatives(
    occurrences: &[WindowOccurrence],
    tokenized: &[(&str, Vec<Token>)],
) -> Vec<WindowOccurrence> {
    let mut representatives = Vec::new();
    for occurrence in occurrences {
        let path = tokenized[occurrence.file_idx].0;
        if representatives
            .iter()
            .all(|representative: &WindowOccurrence| tokenized[representative.file_idx].0 != path)
        {
            representatives.push(*occurrence);
            if representatives.len() == 2 {
                break;
            }
        }
    }
    representatives
}

fn maybe_record_cross_file_match(
    left: WindowOccurrence,
    right: WindowOccurrence,
    tokenized: &[(&str, Vec<Token>)],
    matches: &mut Vec<CrossFileCloneMatch>,
    covered_pairs: &mut HashMap<(usize, usize), Vec<((usize, usize), (usize, usize))>>,
) {
    if left.file_idx == right.file_idx || tokenized[left.file_idx].0 == tokenized[right.file_idx].0
    {
        return;
    }

    let (file_a, start_a, file_b, start_b) = if left.file_idx < right.file_idx {
        (left.file_idx, left.start, right.file_idx, right.start)
    } else {
        (right.file_idx, right.start, left.file_idx, left.start)
    };
    let tokens_a = &tokenized[file_a].1;
    let tokens_b = &tokenized[file_b].1;

    if !cross_windows_equal(tokens_a, start_a, tokens_b, start_b, MIN_CLONE_TOKENS) {
        return;
    }

    let (start_a, start_b, token_len) =
        extend_cross_file_match(tokens_a, start_a, tokens_b, start_b, MIN_CLONE_TOKENS);
    let pair_key = (file_a, file_b);
    if let Some(covered) = covered_pairs.get(&pair_key) {
        if is_pair_covered(start_a, start_b, covered) {
            return;
        }
    }

    matches.push(CrossFileCloneMatch {
        file_a,
        file_b,
        start_a,
        start_b,
        token_len,
    });
    covered_pairs.entry(pair_key).or_default().push((
        (start_a, start_a + token_len),
        (start_b, start_b + token_len),
    ));
}

fn detect_clone_matches(tokens: &[Token]) -> Vec<CloneMatch> {
    if tokens.len() < MIN_CLONE_TOKENS * 2 {
        return Vec::new();
    }

    let window_hashes = rolling_window_hashes(tokens, MIN_CLONE_TOKENS);
    let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
    for (start, hash) in window_hashes.into_iter().enumerate() {
        buckets.entry(hash).or_default().push(start);
    }

    let mut matches = Vec::new();
    let mut covered_pairs: Vec<((usize, usize), (usize, usize))> = Vec::new();
    let mut hashes: Vec<u64> = buckets.keys().copied().collect();
    hashes.sort_unstable();

    'outer: for hash in hashes {
        let Some(starts) = buckets.get(&hash) else {
            continue;
        };
        if starts.len() < 2 {
            continue;
        }

        for i in 0..starts.len() {
            for j in (i + 1)..starts.len() {
                let start_a = starts[i];
                let start_b = starts[j];

                if start_b < start_a + MIN_CLONE_TOKENS {
                    continue;
                }
                if !windows_equal(tokens, start_a, start_b, MIN_CLONE_TOKENS) {
                    continue;
                }
                if is_pair_covered(start_a, start_b, &covered_pairs) {
                    continue;
                }

                let token_len = extend_match(tokens, start_a, start_b, MIN_CLONE_TOKENS);
                matches.push(CloneMatch {
                    start_a,
                    start_b,
                    token_len,
                });
                covered_pairs.push((
                    (start_a, start_a + token_len),
                    (start_b, start_b + token_len),
                ));

                if matches.len() >= MAX_CLONE_PAIRS {
                    break 'outer;
                }
            }
        }
    }

    matches.sort_by_key(|m| (tokens[m.start_a].line, tokens[m.start_b].line, m.token_len));
    matches
}

fn rolling_window_hashes(tokens: &[Token], window: usize) -> Vec<u64> {
    if tokens.len() < window || window == 0 {
        return Vec::new();
    }

    let mut hash = 0u64;
    for token in &tokens[..window] {
        hash = mod_add(mod_mul(hash, BASE), token.kind_hash % MODULUS);
    }

    let mut hashes = Vec::with_capacity(tokens.len() - window + 1);
    hashes.push(hash);

    let base_power = mod_pow(BASE, window - 1);
    for start in 1..=(tokens.len() - window) {
        let outgoing = mod_mul(tokens[start - 1].kind_hash % MODULUS, base_power);
        hash = mod_sub(hash, outgoing);
        hash = mod_add(
            mod_mul(hash, BASE),
            tokens[start + window - 1].kind_hash % MODULUS,
        );
        hashes.push(hash);
    }

    hashes
}

fn windows_equal(tokens: &[Token], start_a: usize, start_b: usize, len: usize) -> bool {
    tokens[start_a..start_a + len]
        .iter()
        .zip(&tokens[start_b..start_b + len])
        .all(|(a, b)| a.kind_hash == b.kind_hash)
}

fn cross_windows_equal(
    tokens_a: &[Token],
    start_a: usize,
    tokens_b: &[Token],
    start_b: usize,
    len: usize,
) -> bool {
    if start_a + len > tokens_a.len() || start_b + len > tokens_b.len() {
        return false;
    }

    tokens_a[start_a..start_a + len]
        .iter()
        .zip(&tokens_b[start_b..start_b + len])
        .all(|(a, b)| a.kind_hash == b.kind_hash)
}

fn extend_match(tokens: &[Token], start_a: usize, start_b: usize, min_len: usize) -> usize {
    let mut len = min_len;
    while start_b + len < tokens.len()
        && start_a + len < start_b
        && tokens[start_a + len].kind_hash == tokens[start_b + len].kind_hash
    {
        len += 1;
    }
    len
}

fn extend_cross_file_match(
    tokens_a: &[Token],
    start_a: usize,
    tokens_b: &[Token],
    start_b: usize,
    min_len: usize,
) -> (usize, usize, usize) {
    let mut back_len = 0;
    while start_a > back_len
        && start_b > back_len
        && tokens_a[start_a - back_len - 1].kind_hash == tokens_b[start_b - back_len - 1].kind_hash
    {
        back_len += 1;
    }

    let start_a = start_a - back_len;
    let start_b = start_b - back_len;
    let mut len = min_len + back_len;
    while start_a + len < tokens_a.len()
        && start_b + len < tokens_b.len()
        && tokens_a[start_a + len].kind_hash == tokens_b[start_b + len].kind_hash
    {
        len += 1;
    }
    (start_a, start_b, len)
}

fn is_pair_covered(
    start_a: usize,
    start_b: usize,
    covered_pairs: &[((usize, usize), (usize, usize))],
) -> bool {
    covered_pairs
        .iter()
        .any(|((a0, a1), (b0, b1))| (*a0..*a1).contains(&start_a) && (*b0..*b1).contains(&start_b))
}

fn mod_mul(a: u64, b: u64) -> u64 {
    ((u128::from(a) * u128::from(b)) % u128::from(MODULUS)) as u64
}

fn mod_add(a: u64, b: u64) -> u64 {
    let sum = u128::from(a) + u128::from(b);
    (sum % u128::from(MODULUS)) as u64
}

fn mod_sub(a: u64, b: u64) -> u64 {
    if a >= b {
        a - b
    } else {
        MODULUS - (b - a)
    }
}

fn mod_pow(mut base: u64, mut exp: usize) -> u64 {
    let mut result = 1u64;
    while exp > 0 {
        if exp % 2 == 1 {
            result = mod_mul(result, base);
        }
        base = mod_mul(base, base);
        exp /= 2;
    }
    result
}

pub fn duplication_pct(lang: &str, text: &str) -> f64 {
    let tokens = tokenize(lang, text);
    if tokens.is_empty() {
        return 0.0;
    }

    let matches = detect_clone_matches(&tokens);
    if matches.is_empty() {
        return 0.0;
    }

    let mut duplicated = vec![false; tokens.len()];
    for m in matches {
        for idx in m.start_a..(m.start_a + m.token_len).min(tokens.len()) {
            duplicated[idx] = true;
        }
        for idx in m.start_b..(m.start_b + m.token_len).min(tokens.len()) {
            duplicated[idx] = true;
        }
    }

    let duplicated_count = duplicated.into_iter().filter(|is_dup| *is_dup).count();
    duplicated_count as f64 / tokens.len() as f64
}

pub fn cross_file_duplication_pct(files: &[(String, String, String)]) -> f64 {
    let tokenized: Vec<(&str, Vec<Token>)> = files
        .iter()
        .map(|(path, lang, text)| (path.as_str(), tokenize(lang, text)))
        .collect();
    let total_tokens: usize = tokenized.iter().map(|(_, tokens)| tokens.len()).sum();
    if total_tokens == 0 {
        return 0.0;
    }

    let duplicated = cross_file_duplicated_token_bitmaps(&tokenized);

    let duplicated_count: usize = duplicated
        .into_iter()
        .map(|file| file.into_iter().filter(|is_dup| *is_dup).count())
        .sum();
    duplicated_count as f64 / total_tokens as f64
}

pub fn per_file_duplicated_token_counts(
    files: &[(String, String, String)],
) -> HashMap<String, usize> {
    let tokenized: Vec<(&str, Vec<Token>)> = files
        .iter()
        .map(|(path, lang, text)| (path.as_str(), tokenize(lang, text)))
        .collect();
    let duplicated = cross_file_duplicated_token_bitmaps(&tokenized);
    let mut counts = HashMap::new();
    for ((path, _), duplicated) in tokenized.iter().zip(duplicated) {
        let duplicated_count = duplicated.into_iter().filter(|is_dup| *is_dup).count();
        counts
            .entry((*path).to_string())
            .and_modify(|count| *count += duplicated_count)
            .or_insert(duplicated_count);
    }
    counts
}

fn cross_file_duplicated_token_bitmaps(tokenized: &[(&str, Vec<Token>)]) -> Vec<Vec<bool>> {
    let mut duplicated: Vec<Vec<bool>> = tokenized
        .iter()
        .map(|(_, tokens)| vec![false; tokens.len()])
        .collect();
    for m in detect_cross_file_clone_matches(tokenized) {
        for idx in m.start_a..(m.start_a + m.token_len).min(duplicated[m.file_a].len()) {
            duplicated[m.file_a][idx] = true;
        }
        for idx in m.start_b..(m.start_b + m.token_len).min(duplicated[m.file_b].len()) {
            duplicated[m.file_b][idx] = true;
        }
    }
    duplicated
}

pub fn dry_violation(lang: &str, text: &str) -> bool {
    dry_duplication_pct_violates(duplication_pct(lang, text))
}

pub(crate) fn dry_duplication_pct_violates(duplication_pct: f64) -> bool {
    duplication_pct >= DRY_DUPLICATION_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duplicated_rust_source() -> String {
        let block_a = r#"
fn compute_alpha(input: i32) -> i32 {
    let mut total = 0;
    let mut current = input;
    for idx in 0..25 {
        total += idx;
        current += total;
        if current % 2 == 0 {
            total += current / 2;
        } else {
            total += current * 3;
        }
        total -= idx / 2;
        current += 1;
    }
    total += current;
    total += input;
    total += current - input;
    total += input * 2;
    total += current / 3;
    total += input % 5;
    total += current + input;
    total -= input / 2;
    total += current * input;
    total
}
"#;
        let block_b = r#"
fn compute_beta(value: i32) -> i32 {
    let mut total = 0;
    let mut current = value;
    for idx in 0..25 {
        total += idx;
        current += total;
        if current % 2 == 0 {
            total += current / 2;
        } else {
            total += current * 3;
        }
        total -= idx / 2;
        current += 1;
    }
    total += current;
    total += value;
    total += current - value;
    total += value * 2;
    total += current / 3;
    total += value % 5;
    total += current + value;
    total -= value / 2;
    total += current * value;
    total
}
"#;
        format!("{block_a}\n{block_b}")
    }

    fn cross_file_rust_source(name: &str, param: &str) -> String {
        format!(
            r#"
fn {name}({param}: i32) -> i32 {{
    let mut total = 0;
    let mut current = {param};
    for idx in 0..25 {{
        total += idx;
        current += total;
        if current % 2 == 0 {{
            total += current / 2;
        }} else {{
            total += current * 3;
        }}
        total -= idx / 2;
        current += 1;
    }}
    total += current;
    total += {param};
    total += current - {param};
    total += {param} * 2;
    total += current / 3;
    total += {param} % 5;
    total += current + {param};
    total -= {param} / 2;
    total += current * {param};
    total
}}
"#
        )
    }

    fn boundary_unique_source(name: &str, variant: usize) -> String {
        let mut src = String::new();
        src.push_str(&format!("fn {name}_first(input: i32) -> i32 {{\n"));
        match variant {
            0 => {
                src.push_str("    let mut total = input;\n");
                for _ in 0..20 {
                    src.push_str("    total = total + input;\n");
                    src.push_str("    total = total - input;\n");
                }
            }
            _ => {
                src.push_str("    let mut total = input * input;\n");
                for _ in 0..20 {
                    src.push_str("    if total > input {\n        total = total / 2;\n    } else {\n        total = total * 3;\n    }\n");
                }
            }
        }
        src.push_str("    total\n}\n\n");
        src.push_str(&format!("fn {name}_second(input: i32) -> i32 {{\n"));
        match variant {
            0 => {
                src.push_str("    let mut total = input;\n");
                for _ in 0..20 {
                    src.push_str("    total = total + input;\n");
                    src.push_str("    total = total - input;\n");
                }
            }
            _ => {
                src.push_str("    let mut total = input * input;\n");
                for _ in 0..20 {
                    src.push_str("    if total > input {\n        total = total / 2;\n    } else {\n        total = total * 3;\n    }\n");
                }
            }
        }
        src.push_str("    total\n}\n");
        src
    }

    fn shared_block_source(name: &str, prefix: &str, suffix: &str, lines: usize) -> String {
        let mut src = format!("fn {name}(input: i32) -> i32 {{\n    {prefix}\n");
        for _ in 0..lines {
            src.push_str("    total = total + input;\n");
        }
        src.push_str(&format!("    {suffix}\n    total\n}}\n"));
        src
    }

    #[test]
    fn duplicated_rust_blocks_produce_clone_pair_and_percentage() {
        let src = duplicated_rust_source();
        let clones = detect_clones("rust", &src);
        assert!(!clones.is_empty(), "expected at least one clone pair");
        assert!(
            clones
                .iter()
                .any(|clone| clone.token_len >= MIN_CLONE_TOKENS),
            "got {clones:?}"
        );
        assert!(clones
            .iter()
            .all(|clone| { clone.a_end_line >= clone.line_a && clone.b_end_line >= clone.line_b }));
        assert!(duplication_pct("rust", &src) > 0.0);
    }

    #[test]
    fn short_unique_source_has_no_duplication() {
        let src = "fn unique(value: i32) -> i32 { value + 1 }\n";
        assert!(detect_clones("rust", src).is_empty());
        assert_eq!(duplication_pct("rust", src), 0.0);
    }

    #[test]
    fn tokenization_collapses_different_identifiers() {
        let x_tokens = tokenize("rust", "fn a() { let x = 1; }\n");
        let y_tokens = tokenize("rust", "fn b() { let y = 1; }\n");
        let x_hashes: Vec<u64> = x_tokens.iter().map(|token| token.kind_hash).collect();
        let y_hashes: Vec<u64> = y_tokens.iter().map(|token| token.kind_hash).collect();
        assert_eq!(x_hashes, y_hashes);
    }

    #[test]
    fn cross_file_identical_blocks_produce_clone_pair() {
        let files = vec![
            (
                "a.rs".to_string(),
                "rust".to_string(),
                cross_file_rust_source("compute_alpha", "input"),
            ),
            (
                "b.rs".to_string(),
                "rust".to_string(),
                cross_file_rust_source("compute_beta", "value"),
            ),
        ];
        let clones = detect_cross_file_clones(&files);
        assert!(
            clones.iter().any(|clone| clone.file_a == "a.rs"
                && clone.file_b == "b.rs"
                && clone.token_len >= MIN_CLONE_TOKENS),
            "got {clones:?}"
        );
        assert!(cross_file_duplication_pct(&files) > 0.0);
    }

    #[test]
    fn cross_file_unique_sources_produce_no_clone_pair() {
        let files = vec![
            (
                "a.rs".to_string(),
                "rust".to_string(),
                "fn unique_a(value: i32) -> i32 { value + 1 }\n".to_string(),
            ),
            (
                "b.rs".to_string(),
                "rust".to_string(),
                "fn unique_b(value: i32) -> i32 { value * 3 }\n".to_string(),
            ),
        ];
        assert!(detect_cross_file_clones(&files).is_empty());
        assert_eq!(cross_file_duplication_pct(&files), 0.0);
    }

    #[test]
    fn same_file_content_under_different_paths_is_cross_file_clone() {
        let src = cross_file_rust_source("compute_alpha", "input");
        let files = vec![
            ("a.rs".to_string(), "rust".to_string(), src.clone()),
            ("copy.rs".to_string(), "rust".to_string(), src),
        ];
        let clones = detect_cross_file_clones(&files);
        assert!(
            clones.iter().any(|clone| clone.file_a == "a.rs"
                && clone.file_b == "copy.rs"
                && clone.token_len >= MIN_CLONE_TOKENS),
            "got {clones:?}"
        );
        assert_eq!(cross_file_duplication_pct(&files), 1.0);
    }

    #[test]
    fn per_file_duplicated_tokens_count_overlapping_regions_once() {
        let src_a = cross_file_rust_source("compute_alpha", "input");
        let src_b = cross_file_rust_source("compute_beta", "value");
        let src_c = cross_file_rust_source("compute_gamma", "item");
        let files = vec![
            ("a.rs".to_string(), "rust".to_string(), src_a.clone()),
            ("b.rs".to_string(), "rust".to_string(), src_b),
            ("c.rs".to_string(), "rust".to_string(), src_c),
        ];
        let clones = detect_cross_file_clones(&files);
        let a_pair_token_sum = clones
            .iter()
            .filter(|clone| clone.file_a == "a.rs" || clone.file_b == "a.rs")
            .map(|clone| clone.token_len)
            .sum::<usize>();
        let a_total = tokenize("rust", &src_a).len();

        let counts = per_file_duplicated_token_counts(&files);

        assert!(a_pair_token_sum > a_total, "got clones {clones:?}");
        assert_eq!(counts.get("a.rs").copied(), Some(a_total));
        assert_eq!(cross_file_duplication_pct(&files), 1.0);
    }

    #[test]
    fn cross_file_matches_do_not_span_file_boundaries() {
        let mut tokens_a: Vec<Token> = (0..MIN_CLONE_TOKENS)
            .map(|idx| Token {
                kind_hash: 10_000 + idx as u64,
                line: idx + 1,
            })
            .collect();
        tokens_a.extend((0..MIN_CLONE_TOKENS).map(|idx| Token {
            kind_hash: idx as u64,
            line: MIN_CLONE_TOKENS + idx + 1,
        }));
        let mut tokens_b: Vec<Token> = (0..MIN_CLONE_TOKENS)
            .map(|idx| Token {
                kind_hash: idx as u64,
                line: idx + 1,
            })
            .collect();
        tokens_b.extend((0..MIN_CLONE_TOKENS).map(|idx| Token {
            kind_hash: 20_000 + idx as u64,
            line: MIN_CLONE_TOKENS + idx + 1,
        }));
        let mut raw_combined = tokens_a.clone();
        raw_combined.extend_from_slice(&tokens_b);
        assert!(!detect_clone_matches(&raw_combined).is_empty());

        let tokenized = vec![("a.rs", tokens_a), ("b.rs", tokens_b)];
        let matches = detect_cross_file_clone_matches(&tokenized);
        assert!(matches.iter().all(|m| {
            m.start_a + m.token_len <= tokenized[m.file_a].1.len()
                && m.start_b + m.token_len <= tokenized[m.file_b].1.len()
        }));

        let files = vec![
            (
                "a.rs".to_string(),
                "rust".to_string(),
                boundary_unique_source("boundary_a", 0),
            ),
            (
                "b.rs".to_string(),
                "rust".to_string(),
                boundary_unique_source("boundary_b", 1),
            ),
        ];
        assert!(files
            .iter()
            .all(|(_, lang, text)| tokenize(lang, text).len() > MIN_CLONE_TOKENS));
        assert!(detect_cross_file_clones(&files).is_empty());
    }

    #[test]
    fn cross_file_clone_pair_reports_exact_lines() {
        let shared_lines = 18;
        let files = vec![
            (
                "a.rs".to_string(),
                "rust".to_string(),
                shared_block_source(
                    "with_shared_a",
                    "let mut total = input;",
                    "total = total - input;",
                    shared_lines,
                ),
            ),
            (
                "b.rs".to_string(),
                "rust".to_string(),
                shared_block_source(
                    "with_shared_b",
                    "let mut total = input * input;",
                    "total = total / 2;",
                    shared_lines,
                ),
            ),
        ];
        let clones = detect_cross_file_clones(&files);
        let clone = clones
            .iter()
            .find(|clone| clone.file_a == "a.rs" && clone.file_b == "b.rs")
            .expect("expected cross-file clone");
        assert_eq!(clone.line_a, clone.a_start_line);
        assert_eq!(clone.line_b, clone.b_start_line);
        assert!(
            clone.a_start_line >= 1 && clone.a_start_line <= 3,
            "a_start_line={}",
            clone.a_start_line
        );
        assert!(
            clone.a_end_line >= 18 && clone.a_end_line >= clone.a_start_line,
            "a_end_line={}",
            clone.a_end_line
        );
        assert!(
            clone.b_start_line >= 1 && clone.b_start_line <= 3,
            "b_start_line={}",
            clone.b_start_line
        );
        assert!(
            clone.b_end_line >= 18 && clone.b_end_line >= clone.b_start_line,
            "b_end_line={}",
            clone.b_end_line
        );
        assert!(clone.token_len >= MIN_CLONE_TOKENS);
    }

    #[test]
    fn clone_beyond_old_prefix_cap_is_found() {
        let mut files: Vec<(String, String, String)> = (0..80)
            .map(|idx| {
                (
                    format!("filler_{idx}.rs"),
                    "rust".to_string(),
                    format!("fn filler_{idx}() -> i32 {{ {idx} }}\n"),
                )
            })
            .collect();
        files[70] = (
            "tail_a.rs".to_string(),
            "rust".to_string(),
            cross_file_rust_source("compute_tail_a", "input"),
        );
        files[75] = (
            "tail_b.rs".to_string(),
            "rust".to_string(),
            cross_file_rust_source("compute_tail_b", "value"),
        );
        let processed_before_old_pair: usize =
            (0..70).map(|i| files.len() - i - 1).sum::<usize>() + 5;
        assert!(processed_before_old_pair > 2000);

        let clones = detect_cross_file_clones(&files);

        assert!(
            clones.iter().any(|clone| clone.file_a == "tail_a.rs"
                && clone.file_b == "tail_b.rs"
                && clone.token_len >= MIN_CLONE_TOKENS),
            "got {clones:?}"
        );
    }

    #[test]
    fn cross_file_analysis_matches_individual_passes() {
        let files = vec![
            (
                "a.rs".to_string(),
                "rust".to_string(),
                cross_file_rust_source("compute_alpha", "input"),
            ),
            (
                "b.rs".to_string(),
                "rust".to_string(),
                cross_file_rust_source("compute_beta", "value"),
            ),
            (
                "unique.rs".to_string(),
                "rust".to_string(),
                "fn unique(value: i32) -> i32 { value + 1 }\n".to_string(),
            ),
        ];

        let combined = cross_file_analysis(&files);

        assert_eq!(combined.clones, detect_cross_file_clones(&files));
        assert_eq!(combined.duplication_pct, cross_file_duplication_pct(&files));
        assert_eq!(
            combined.duplicated_tokens_by_path,
            per_file_duplicated_token_counts(&files)
        );
        for (path, lang, text) in &files {
            assert_eq!(
                combined.tokens_by_path.get(path).copied(),
                Some(tokenize(lang, text).len()),
                "token totals must match for {path}"
            );
        }
    }

    #[test]
    fn output_cap_does_not_hide_pct() {
        let src = cross_file_rust_source("compute_shared", "input");
        let files: Vec<(String, String, String)> = (0..(MAX_CLONE_PAIRS + 5))
            .map(|idx| (format!("clone_{idx}.rs"), "rust".to_string(), src.clone()))
            .collect();

        let clones = detect_cross_file_clones(&files);
        let pct = cross_file_duplication_pct(&files);

        assert_eq!(clones.len(), MAX_CLONE_PAIRS);
        assert_eq!(pct, 1.0);
    }
}
