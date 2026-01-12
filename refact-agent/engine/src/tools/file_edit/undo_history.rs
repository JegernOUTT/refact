use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const MAX_ENTRIES_PER_FILE: usize = 20;
const MAX_TOTAL_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone)]
pub struct UndoEntry {
    pub content: String,
    pub timestamp: Instant,
}

type UndoMap = HashMap<PathBuf, Vec<UndoEntry>>;

static UNDO_HISTORY: OnceLock<Mutex<UndoMap>> = OnceLock::new();

pub fn get_undo_history() -> &'static Mutex<UndoMap> {
    UNDO_HISTORY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn record_before_edit(path: &PathBuf, content: &str) {
    let history = get_undo_history();
    let mut h = history.lock().unwrap();

    let entries = h.entry(path.clone()).or_insert_with(Vec::new);

    if entries.last().map(|e| e.content.as_str()) == Some(content) {
        return;
    }

    entries.push(UndoEntry {
        content: content.to_string(),
        timestamp: Instant::now(),
    });

    if entries.len() > MAX_ENTRIES_PER_FILE {
        entries.remove(0);
    }

    let total_bytes: usize = h
        .values()
        .flat_map(|v| v.iter())
        .map(|e| e.content.len())
        .sum();

    if total_bytes > MAX_TOTAL_BYTES {
        prune_oldest(&mut h);
    }
}

fn prune_oldest(h: &mut UndoMap) {
    let mut all_entries: Vec<(PathBuf, usize, Instant)> = Vec::new();
    for (path, entries) in h.iter() {
        for (idx, entry) in entries.iter().enumerate() {
            all_entries.push((path.clone(), idx, entry.timestamp));
        }
    }
    all_entries.sort_by_key(|(_, _, ts)| *ts);

    if let Some((path, idx, _)) = all_entries.first() {
        if let Some(entries) = h.get_mut(path) {
            if *idx < entries.len() {
                entries.remove(*idx);
            }
            if entries.is_empty() {
                h.remove(path);
            }
        }
    }
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

        for i in 0..25 {
            record_before_edit(&path, &format!("version{}", i));
        }

        let h = history.lock().unwrap();
        let entries = h.get(&path).unwrap();
        assert!(entries.len() <= MAX_ENTRIES_PER_FILE);
    }
}
