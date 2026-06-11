// Adapted from openai/codex codex-rs/tui, Apache-2.0.

use std::iter::Peekable;
use std::ops::Range;

use pulldown_cmark::Event;

pub struct DecodedTextMerge<I: Iterator> {
    iter: Peekable<I>,
}

impl<I: Iterator> DecodedTextMerge<I> {
    pub fn new(iter: I) -> Self {
        Self {
            iter: iter.peekable(),
        }
    }
}

impl<'a, I> Iterator for DecodedTextMerge<I>
where
    I: Iterator<Item = (Event<'a>, Range<usize>)>,
{
    type Item = (Event<'a>, Range<usize>);

    fn next(&mut self) -> Option<Self::Item> {
        let (event, mut range) = self.iter.next()?;
        let Event::Text(text) = event else {
            return Some((event, range));
        };
        if !matches!(self.iter.peek(), Some((Event::Text(_), _))) {
            return Some((Event::Text(text), range));
        }

        let mut merged = text.into_string();
        while matches!(self.iter.peek(), Some((Event::Text(_), _))) {
            let Some((Event::Text(text), next_range)) = self.iter.next() else {
                break;
            };
            merged.push_str(&text);
            range.end = next_range.end;
        }
        Some((Event::Text(merged.into()), range))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::CowStr;

    #[test]
    fn merges_adjacent_text_events() {
        let events = vec![
            (Event::Text(CowStr::from("hel")), 0..3),
            (Event::Text(CowStr::from("lo")), 3..5),
            (Event::SoftBreak, 5..6),
            (Event::Text(CowStr::from("world")), 6..11),
        ];
        let merged: Vec<_> = DecodedTextMerge::new(events.into_iter()).collect();
        assert_eq!(merged.len(), 3);
        assert!(
            matches!(&merged[0], (Event::Text(text), range) if text.as_ref() == "hello" && *range == (0..5))
        );
    }
}
