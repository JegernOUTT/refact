pub use refact_buddy_core::types::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buddy_page_conductor() {
        let page_json = serde_json::to_string(&BuddyPage::Conductor).unwrap();
        let page_back: BuddyPage = serde_json::from_str(&page_json).unwrap();

        assert_eq!(page_json, r#"{"type":"conductor"}"#);
        assert_eq!(page_back, BuddyPage::Conductor);
    }
}
