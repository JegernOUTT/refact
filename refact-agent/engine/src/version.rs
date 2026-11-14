use shadow_rs::shadow;

shadow!(build);

pub const VERSION: &str = env!("REFACT_ENGINE_VERSION");

static BUILD_INFO_FRAME: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/runtime-build-info.bin"));

pub fn build_info_json() -> &'static str {
    let frame = std::hint::black_box(BUILD_INFO_FRAME);
    std::str::from_utf8(&frame[22..frame.len() - 23]).expect("build identity JSON")
}
