pub fn usable_content_width(total_width: usize, reserved_cols: usize) -> Option<usize> {
    total_width
        .checked_sub(reserved_cols)
        .filter(|remaining| *remaining > 0)
}

pub fn usable_content_width_u16(total_width: u16, reserved_cols: u16) -> Option<usize> {
    usable_content_width(usize::from(total_width), usize::from(reserved_cols))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usable_content_width_returns_none_when_reserved_exhausts_width() {
        assert_eq!(usable_content_width(0, 0), None);
        assert_eq!(usable_content_width(2, 2), None);
        assert_eq!(usable_content_width(3, 4), None);
        assert_eq!(usable_content_width(5, 4), Some(1));
    }

    #[test]
    fn usable_content_width_u16_matches_usize_variant() {
        assert_eq!(usable_content_width_u16(2, 2), None);
        assert_eq!(usable_content_width_u16(5, 4), Some(1));
    }
}
