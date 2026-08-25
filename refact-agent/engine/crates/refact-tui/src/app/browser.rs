use super::*;

impl App {
    pub(super) fn open_browser_command(&mut self) {
        self.open_browser_surface();
    }
}

pub(super) fn append_inline_image(
    app: &mut App,
    buffer: &ratatui::buffer::Buffer,
    images: &mut Vec<crate::terminal_image::InlineImage>,
) {
    let browser_state = app.browser_state().clone();
    if let Some(image) = app.browser_surface.as_mut().and_then(|surface| {
        surface.next_inline_frame(
            &browser_state,
            buffer,
            crate::terminal_probe::image_protocol_from_env(),
        )
    }) {
        images.push(image);
    }
}
