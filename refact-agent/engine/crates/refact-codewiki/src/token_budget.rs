pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        (chars + 3) / 4
    }
}

pub fn trim_to_budget(text: &str, remaining: usize) -> String {
    if estimate_tokens(text) <= remaining {
        return text.to_string();
    }

    let suffix = "...[truncated]";
    let suffix_tokens = estimate_tokens(suffix);

    if remaining < suffix_tokens {
        return String::new();
    }

    let max_chars = (remaining - suffix_tokens) * 4;
    let mut result: String = text.chars().take(max_chars).collect();
    result.push_str(suffix);
    result
}

pub fn items_within_budget<T, F: Fn(&T) -> usize>(
    items: Vec<T>,
    used: usize,
    budget: usize,
    cost_fn: F,
) -> (Vec<T>, usize) {
    let mut selected = Vec::new();
    let mut used = used;

    for item in items {
        let cost = cost_fn(&item);
        if used + cost <= budget {
            selected.push(item);
            used += cost;
        } else {
            break;
        }
    }

    (selected, used)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_tokens_using_four_chars_per_token() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abc"), 1);
        assert_eq!(estimate_tokens("12345678"), 2);
    }

    #[test]
    fn trim_to_budget_returns_text_unchanged_when_it_fits() {
        let text = "short text";

        assert_eq!(trim_to_budget(text, estimate_tokens(text)), text);
    }

    #[test]
    fn trim_to_budget_truncates_and_appends_suffix() {
        let text = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let remaining = 5;
        let result = trim_to_budget(text, remaining);

        assert!(result.ends_with("...[truncated]"));
        assert_ne!(result, text);
        assert!(estimate_tokens(&result) <= remaining);
    }

    #[test]
    fn trim_to_budget_handles_unicode_safely() {
        let text = "😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀😀";
        let remaining = 4;
        let result = trim_to_budget(text, remaining);

        assert!(result.ends_with("...[truncated]"));
        assert!(estimate_tokens(&result) <= remaining);
    }

    #[test]
    fn trim_to_budget_returns_suffix_when_remaining_matches_suffix_tokens() {
        assert_eq!(
            trim_to_budget("abcdefghijklmnopqrstuvwxyz", 4),
            "...[truncated]"
        );
    }

    #[test]
    fn trim_to_budget_returns_empty_when_remaining_is_less_than_suffix_tokens() {
        let text = "abcdefghijklmnopqrstuvwxyz";

        for remaining in [1, 2, 3] {
            let result = trim_to_budget(text, remaining);

            assert_eq!(result, "");
            assert!(estimate_tokens(&result) <= remaining);
        }
    }

    #[test]
    fn trim_to_budget_returns_empty_when_remaining_is_zero() {
        assert_eq!(trim_to_budget("abc", 0), "");
    }

    #[test]
    fn items_within_budget_keeps_exact_fit() {
        let items = vec![1, 2, 3, 4];
        let (selected, used) = items_within_budget(items, 1, 7, |item| *item);

        assert_eq!(selected, vec![1, 2, 3]);
        assert_eq!(used, 7);
    }

    #[test]
    fn items_within_budget_breaks_at_first_item_that_does_not_fit() {
        let items = vec![1, 5, 1];
        let (selected, used) = items_within_budget(items, 0, 5, |item| *item);

        assert_eq!(selected, vec![1]);
        assert_eq!(used, 1);
    }
}
