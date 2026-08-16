use super::ObservationStatus;

pub(super) fn status(requested: bool) -> ObservationStatus {
    let reason = if requested {
        "macOS syscall observation unavailable"
    } else {
        "disabled"
    };
    ObservationStatus::Unavailable(reason.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_observation_reports_exact_unavailable_reason() {
        assert_eq!(
            status(true),
            ObservationStatus::Unavailable("macOS syscall observation unavailable".to_string())
        );
    }

    #[test]
    fn disabled_observation_remains_disabled() {
        assert_eq!(
            status(false),
            ObservationStatus::Unavailable("disabled".to_string())
        );
    }
}
