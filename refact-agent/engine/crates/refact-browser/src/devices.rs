use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::context_state::ViewportState;

const DEVICE_DESCRIPTORS: &str = include_str!("device_descriptors.json");

const DEVICE_ALIASES: &[(&str, &str)] = &[
    ("mobile", "Pixel 7"),
    ("tablet", "Galaxy Tab S4"),
    ("desktop", "Desktop Chrome"),
];

const MAX_DEVICE_SUGGESTIONS: usize = 5;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceDescriptorSource {
    user_agent: String,
    viewport: DeviceViewportSource,
    device_scale_factor: f64,
    is_mobile: bool,
    has_touch: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceViewportSource {
    width: u32,
    height: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceDescriptor {
    pub name: String,
    pub user_agent: String,
    pub viewport: ViewportState,
}

impl DeviceDescriptor {
    pub fn summary(&self) -> String {
        format!(
            "{} ({}x{} @{}x{}{})",
            self.name,
            self.viewport.width,
            self.viewport.height,
            self.viewport.device_scale_factor,
            if self.viewport.is_mobile {
                " mobile"
            } else {
                " desktop"
            },
            if self.viewport.has_touch {
                " touch"
            } else {
                ""
            }
        )
    }
}

fn parse_registry(source: &str) -> Result<Vec<DeviceDescriptor>, String> {
    let parsed: BTreeMap<String, DeviceDescriptorSource> = serde_json::from_str(source)
        .map_err(|error| format!("Failed to parse vendored device descriptors: {error}"))?;
    Ok(parsed
        .into_iter()
        .map(|(name, source)| DeviceDescriptor {
            name,
            user_agent: source.user_agent,
            viewport: ViewportState {
                width: source.viewport.width,
                height: source.viewport.height,
                device_scale_factor: source.device_scale_factor,
                is_mobile: source.is_mobile,
                has_touch: source.has_touch,
            },
        })
        .collect())
}

pub fn registry() -> &'static [DeviceDescriptor] {
    static REGISTRY: OnceLock<Vec<DeviceDescriptor>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        parse_registry(DEVICE_DESCRIPTORS).expect("vendored device_descriptors.json must parse")
    })
}

pub fn resolve_alias(name: &str) -> Option<&'static str> {
    let lowered = name.trim().to_ascii_lowercase();
    DEVICE_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lowered)
        .map(|(_, target)| *target)
}

pub fn lookup(name: &str) -> Result<&'static DeviceDescriptor, String> {
    let requested = resolve_alias(name).unwrap_or(name.trim());
    registry()
        .iter()
        .find(|device| device.name == requested)
        .or_else(|| {
            registry()
                .iter()
                .find(|device| device.name.eq_ignore_ascii_case(requested))
        })
        .ok_or_else(|| unknown_device_error(name))
}

pub fn list(filter: Option<&str>) -> Vec<&'static str> {
    let needle = filter.map(|value| value.trim().to_ascii_lowercase());
    registry()
        .iter()
        .filter(|device| match needle.as_deref() {
            Some(needle) if !needle.is_empty() => device.name.to_ascii_lowercase().contains(needle),
            _ => true,
        })
        .map(|device| device.name.as_str())
        .collect()
}

fn unknown_device_error(name: &str) -> String {
    let suggestions = closest_names(name).join(", ");
    format!(
        "Unknown device '{}'. Closest names: {}. Use list_devices for all {} names plus the mobile, tablet, and desktop aliases",
        name.trim(),
        suggestions,
        registry().len()
    )
}

fn closest_names(name: &str) -> Vec<&'static str> {
    let lowered = name.trim().to_ascii_lowercase();
    let mut scored = registry()
        .iter()
        .map(|device| {
            let candidate = device.name.to_ascii_lowercase();
            let distance = if candidate.contains(&lowered) && !lowered.is_empty() {
                0
            } else {
                edit_distance(&lowered, &candidate)
            };
            (distance, device.name.as_str())
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(right.1)));
    scored
        .into_iter()
        .take(MAX_DEVICE_SUGGESTIONS)
        .map(|(_, name)| name)
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0usize; right_chars.len() + 1];
    for (row, left_char) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right_chars.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendored_registry_parses_every_descriptor() {
        let devices = parse_registry(DEVICE_DESCRIPTORS).unwrap();
        assert!(
            devices.len() >= 100,
            "expected 100+ devices, got {}",
            devices.len()
        );
        assert_eq!(devices.len(), registry().len());
        assert!(devices.iter().all(|device| !device.user_agent.is_empty()));
        assert!(devices
            .iter()
            .all(|device| device.viewport.width > 0 && device.viewport.height > 0));
    }

    #[test]
    fn named_lookup_returns_combined_viewport_dpr_mobile_touch_and_user_agent() {
        let iphone = lookup("iPhone 13").unwrap();
        assert_eq!(iphone.viewport.width, 390);
        assert_eq!(iphone.viewport.device_scale_factor, 3.0);
        assert!(iphone.viewport.is_mobile && iphone.viewport.has_touch);
        assert!(iphone.user_agent.contains("iPhone"));

        let pixel = lookup("Pixel 7").unwrap();
        assert_eq!(pixel.viewport.width, 412);
        assert_eq!(pixel.viewport.device_scale_factor, 2.625);
        assert!(pixel.user_agent.contains("Pixel 7"));
    }

    #[test]
    fn legacy_aliases_still_resolve_to_representative_registry_entries() {
        assert_eq!(resolve_alias("mobile"), Some("Pixel 7"));
        assert_eq!(resolve_alias("tablet"), Some("Galaxy Tab S4"));
        assert_eq!(resolve_alias("desktop"), Some("Desktop Chrome"));
        assert_eq!(resolve_alias("DESKTOP"), Some("Desktop Chrome"));
        assert_eq!(resolve_alias("iPhone 13"), None);

        assert!(lookup("mobile").unwrap().viewport.is_mobile);
        assert!(lookup("tablet").unwrap().viewport.has_touch);
        let desktop = lookup("desktop").unwrap();
        assert!(!desktop.viewport.is_mobile && !desktop.viewport.has_touch);
    }

    #[test]
    fn unknown_device_error_lists_five_closest_names() {
        let error = lookup("iPhone 113").unwrap_err();
        assert!(error.starts_with("Unknown device 'iPhone 113'."), "{error}");
        assert!(error.contains("iPhone 13"), "{error}");
        assert_eq!(closest_names("iPhone 113").len(), MAX_DEVICE_SUGGESTIONS);
        assert!(error.contains("list_devices"), "{error}");
    }

    #[test]
    fn listing_devices_filters_case_insensitively() {
        let all = list(None);
        assert_eq!(all.len(), registry().len());
        assert!(all.windows(2).all(|pair| pair[0] <= pair[1]));

        let pixels = list(Some("pixel"));
        assert!(!pixels.is_empty());
        assert!(pixels
            .iter()
            .all(|name| name.to_ascii_lowercase().contains("pixel")));
        assert!(pixels.contains(&"Pixel 7"));
        assert!(list(Some("no-such-device")).is_empty());
    }

    #[test]
    fn edit_distance_measures_single_character_edits() {
        assert_eq!(edit_distance("pixel", "pixel"), 0);
        assert_eq!(edit_distance("pixel", "pixe"), 1);
        assert_eq!(edit_distance("pixel", "pixell"), 1);
        assert_eq!(edit_distance("pixel", "pixal"), 1);
    }
}
