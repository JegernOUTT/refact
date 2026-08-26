use std::collections::{HashMap, VecDeque};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ratatui::layout::Rect;

use crate::vendored::terminal_hyperlinks::HyperlinkLine;

pub mod diff;
pub mod highlight;
pub mod line_utils;
pub mod markdown;
pub(crate) mod markdown_table;
pub mod renderable;
pub mod width;
pub mod wrapping;

pub use diff::{
    calculate_add_remove_from_diff, create_diff_summary, display_path_for, is_unified_diff,
    render_unified_diff, DiffLineType,
};
pub use markdown::{render_markdown, render_markdown_with_options, MarkdownRenderer, RenderOptions};

const MAX_RENDER_CACHE_ENTRIES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderCacheKey {
    content_hash: u64,
    width: usize,
    color_enabled: bool,
}

impl RenderCacheKey {
    pub fn new(content: impl Hash, width: usize, color_enabled: bool) -> Self {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Self {
            content_hash: hasher.finish(),
            width,
            color_enabled,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct RenderCache {
    entries: HashMap<RenderCacheKey, Vec<HyperlinkLine>>,
    order: VecDeque<RenderCacheKey>,
    render_count: usize,
}

impl RenderCache {
    pub fn render<F>(&mut self, key: RenderCacheKey, render: F) -> Vec<HyperlinkLine>
    where
        F: FnOnce() -> Vec<HyperlinkLine>,
    {
        if let Some(lines) = self.entries.get(&key).cloned() {
            self.order.retain(|candidate| candidate != &key);
            self.order.push_back(key);
            return lines;
        }
        let lines = render();
        self.entries.insert(key, lines.clone());
        self.order.push_back(key);
        self.render_count += 1;
        while self.entries.len() > MAX_RENDER_CACHE_ENTRIES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
        }
        lines
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }

    pub fn remove(&mut self, key: &RenderCacheKey) {
        self.entries.remove(key);
        self.order.retain(|candidate| candidate != key);
    }

    pub fn render_count(&self) -> usize {
        self.render_count
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insets {
    left: u16,
    top: u16,
    right: u16,
    bottom: u16,
}

impl Insets {
    pub fn tlbr(top: u16, left: u16, bottom: u16, right: u16) -> Self {
        Self {
            top,
            left,
            bottom,
            right,
        }
    }

    pub fn vh(v: u16, h: u16) -> Self {
        Self {
            top: v,
            left: h,
            bottom: v,
            right: h,
        }
    }
}

pub trait RectExt {
    fn inset(&self, insets: Insets) -> Rect;
}

impl RectExt for Rect {
    fn inset(&self, insets: Insets) -> Rect {
        let horizontal = insets.left.saturating_add(insets.right);
        let vertical = insets.top.saturating_add(insets.bottom);
        Rect {
            x: self.x.saturating_add(insets.left),
            y: self.y.saturating_add(insets.top),
            width: self.width.saturating_sub(horizontal),
            height: self.height.saturating_sub(vertical),
        }
    }
}

pub fn color_enabled_from_env() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM")
            .map(|term| term != "dumb")
            .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_cache_promotes_hits_and_retains_recently_used_entries() {
        let mut cache = RenderCache::default();
        let keys = (0..=MAX_RENDER_CACHE_ENTRIES)
            .map(|index| RenderCacheKey::new(index, 80, true))
            .collect::<Vec<_>>();

        for key in keys.iter().take(MAX_RENDER_CACHE_ENTRIES) {
            cache.render(*key, Vec::new);
        }
        cache.render(keys[0], Vec::new);
        cache.render(keys[MAX_RENDER_CACHE_ENTRIES], Vec::new);

        assert!(cache.entries.contains_key(&keys[0]));
        assert!(!cache.entries.contains_key(&keys[1]));
    }
}
