use crate::Cleared;

#[cfg(feature = "test-util")]
pub fn cleared<T>(value: T) -> Cleared<T> {
    Cleared::for_testing(value)
}

#[cfg(all(test, feature = "test-util"))]
mod tests {
    use super::*;

    #[test]
    fn test_utility_constructs_a_cleared_value() {
        let value = cleared("audited elsewhere");

        assert_eq!(*value, "audited elsewhere");
    }
}
