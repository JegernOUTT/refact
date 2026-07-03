pub fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    if chars == 0 {
        0
    } else {
        (chars + 3) / 4
    }
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
