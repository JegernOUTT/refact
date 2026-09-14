use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone)]
pub struct UndoEntry {
    pub content: String,
    pub timestamp: Instant,
    pub existed: bool,
    pub after_digest: Option<String>,
    pub actor: Option<String>,
}

type UndoMap = HashMap<PathBuf, Vec<UndoEntry>>;

static UNDO_HISTORY: OnceLock<Mutex<UndoMap>> = OnceLock::new();

pub fn get_undo_history() -> &'static Mutex<UndoMap> {
    UNDO_HISTORY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn record_before_edit(path: &PathBuf, content: &str) {
    record_edit(path, Some(content), None);
}

pub fn record_edit(path: &PathBuf, before: Option<&str>, after: Option<&str>) {
    let content = before.unwrap_or_default();
    let settings = crate::file_activity::settings();
    if content.len() > settings.max_undo_bytes {
        return;
    }
    let history = get_undo_history();
    let mut h = history.lock().unwrap();
    if !h.contains_key(path) && h.len() >= settings.max_write_records {
        return;
    }

    let entries = h.entry(path.clone()).or_insert_with(Vec::new);

    if after.is_none() && entries.last().map(|e| e.content.as_str()) == Some(content) {
        return;
    }

    entries.push(UndoEntry {
        content: content.to_string(),
        timestamp: Instant::now(),
        existed: before.is_some(),
        after_digest: after.map(crate::file_activity::digest_of),
        actor: crate::file_activity::current_actor(),
    });

    while entries.len() > settings.max_undo_entries_per_file {
        entries.remove(0);
    }

    let mut total_bytes: usize = h
        .values()
        .flat_map(|v| v.iter())
        .map(|e| e.content.len())
        .sum();

    while total_bytes > settings.max_undo_bytes {
        let Some(removed) = prune_oldest(&mut h) else {
            break;
        };
        total_bytes = total_bytes.saturating_sub(removed);
    }
}

fn prune_oldest(h: &mut UndoMap) -> Option<usize> {
    let mut all_entries: Vec<(PathBuf, usize, Instant)> = Vec::new();
    for (path, entries) in h.iter() {
        for (idx, entry) in entries.iter().enumerate() {
            all_entries.push((path.clone(), idx, entry.timestamp));
        }
    }
    all_entries.sort_by_key(|(path, index, ts)| (*ts, path.clone(), *index));

    if let Some((path, idx, _)) = all_entries.first() {
        if let Some(entries) = h.get_mut(path) {
            if *idx < entries.len() {
                let removed = entries.remove(*idx).content.len();
                if entries.is_empty() {
                    h.remove(path);
                }
                return Some(removed);
            }
            if entries.is_empty() {
                h.remove(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_before_edit() {
        let path = PathBuf::from("/tmp/test_undo_1.txt");
        let history = get_undo_history();

        {
            let mut h = history.lock().unwrap();
            h.remove(&path);
        }

        record_before_edit(&path, "version1");
        record_before_edit(&path, "version2");

        let h = history.lock().unwrap();
        let entries = h.get(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "version1");
        assert_eq!(entries[1].content, "version2");
    }

    #[test]
    fn test_record_skips_duplicate() {
        let path = PathBuf::from("/tmp/test_undo_2.txt");
        let history = get_undo_history();

        {
            let mut h = history.lock().unwrap();
            h.remove(&path);
        }

        record_before_edit(&path, "same");
        record_before_edit(&path, "same");

        let h = history.lock().unwrap();
        let entries = h.get(&path).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_max_entries_per_file() {
        let path = PathBuf::from("/tmp/test_undo_3.txt");
        let history = get_undo_history();

        {
            let mut h = history.lock().unwrap();
            h.remove(&path);
        }

        let cap = crate::file_activity::settings().max_undo_entries_per_file;
        for i in 0..cap + 5 {
            record_before_edit(&path, &format!("version{}", i));
        }

        let h = history.lock().unwrap();
        let entries = h.get(&path).unwrap();
        assert!(entries.len() <= cap);
    }
}
