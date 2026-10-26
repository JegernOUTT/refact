use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::context_state::ViewportState;

const DEVICE_DESCRIPTORS: &str = include_str!("device_descriptors.json");

const DEVICE_ALIASES: &[(&str, &str)] = &[
    ("mobile", "Refact Mobile"),
    ("tablet", "Refact Tablet"),
    ("desktop", "Refact Desktop"),
];

const LEGACY_PRESETS: &[(&str, &str, u32, u32, f64, bool, bool)] = &[
    (
        "Refact Desktop",
        "Desktop Chrome",
        1440,
        900,
        2.0,
        false,
        false,
    ),
    ("Refact Mobile", "Pixel 7", 390, 844, 3.0, true, true),
    ("Refact Tablet", "Galaxy Tab S4", 834, 1112, 2.0, true, true),
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
        let mut devices = parse_registry(DEVICE_DESCRIPTORS)
            .expect("vendored device_descriptors.json must parse");
        for (name, ua_source, width, height, device_scale_factor, is_mobile, has_touch) in
            LEGACY_PRESETS
        {
            let user_agent = devices
                .iter()
                .find(|device| device.name == *ua_source)
                .map(|device| device.user_agent.clone())
                .unwrap_or_else(|| "Mozilla/5.0".to_string());
            devices.push(DeviceDescriptor {
                name: (*name).to_string(),
                user_agent,
                viewport: ViewportState {
                    width: *width,
                    height: *height,
                    device_scale_factor: *device_scale_factor,
                    is_mobile: *is_mobile,
                    has_touch: *has_touch,
                },
            });
        }
        devices.sort_by(|left, right| left.name.cmp(&right.name));
        devices
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
