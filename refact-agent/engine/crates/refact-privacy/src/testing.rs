use crate::Cleared;

#[cfg(feature = "test-util")]
pub fn cleared<T>(value: T) -> Cleared<T> {
    Cleared::for_testing(value)
}
