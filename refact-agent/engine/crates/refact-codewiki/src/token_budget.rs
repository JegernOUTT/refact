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
