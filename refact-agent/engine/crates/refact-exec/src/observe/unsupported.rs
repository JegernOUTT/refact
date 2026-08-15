use super::ObservationStatus;

pub(super) fn status(requested: bool) -> ObservationStatus {
    let reason = if requested {
        "backend unavailable"
    } else {
        "disabled"
    };
    ObservationStatus::Unavailable(reason.to_string())
}
