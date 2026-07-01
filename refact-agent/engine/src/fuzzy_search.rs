use std::collections::HashMap;

pub fn fuzzy_search<I>(
    correction_candidate: &String,
    candidates: I,
    top_n: usize,
    separator_chars: &[char],
) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    const FILENAME_WEIGHT: i32 = 3;
    const COMPLETELTY_DROP_DISTANCE: f64 = 0.40;
    const EXCESS_WEIGHT: f64 = 3.0;

    let mut correction_bigram_count: HashMap<(char, char), i32> = HashMap::new();

    let mut correction_candidate_length = 0;
    let mut weight = FILENAME_WEIGHT;
    for window in correction_candidate
        .to_lowercase()
        .chars()
        .collect::<Vec<_>>()
        .windows(2)
        .rev()
    {
        if separator_chars.contains(&window[0]) {
            weight = 1;
        }
        correction_candidate_length += weight;
        *correction_bigram_count
            .entry((window[0], window[1]))
            .or_insert(0) += weight;
    }

    let mut top_n_candidates = Vec::new();

    for candidate in candidates {
        let mut missing_count: i32 = 0;
        let mut excess_count = 0;
        let mut candidate_len = 0;
        let mut bigram_count = correction_bigram_count.clone();

        let mut weight = FILENAME_WEIGHT;
        for window in candidate
            .to_lowercase()
            .chars()
            .collect::<Vec<_>>()
            .windows(2)
            .rev()
        {
            if separator_chars.contains(&window[0]) {
                weight = 1;
            }
            candidate_len += weight;
            if let Some(entry) = bigram_count.get_mut(&(window[0], window[1])) {
                *entry -= weight;
            } else {
                missing_count += weight;
            }
        }

        for (&_, &count) in bigram_count.iter() {
            if count > 0 {
                excess_count += count;
            } else {
                missing_count += -count;
            }
        }

        let distance = (missing_count as f64 + excess_count as f64 * EXCESS_WEIGHT)
            / (correction_candidate_length as f64 + (candidate_len as f64) * EXCESS_WEIGHT);
        if distance < COMPLETELTY_DROP_DISTANCE {
            top_n_candidates.push((candidate, distance));
            top_n_candidates
                .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            if top_n_candidates.len() > top_n {
                top_n_candidates.pop();
            }
        }
    }

    top_n_candidates.into_iter().map(|x| x.0).collect()
}
