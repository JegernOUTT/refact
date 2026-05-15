use std::path::PathBuf;
use ropey::Rope;

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub struct Document {
    pub doc_path: PathBuf,
    pub doc_text: Option<Rope>,
}

impl Document {
    pub fn new(doc_path: &PathBuf) -> Self {
        Self {
            doc_path: doc_path.clone(),
            doc_text: None,
        }
    }

    pub fn update_text(&mut self, text: &String) {
        self.doc_text = Some(Rope::from_str(text));
    }

    pub fn text_as_string(&self) -> Result<String, String> {
        if let Some(r) = &self.doc_text {
            return Ok(r.to_string());
        }
        return Err(format!("no text loaded in {}", self.doc_path.display()));
    }

    pub fn does_text_look_good(&self) -> Result<(), String> {
        assert!(self.doc_text.is_some());
        let r = self.doc_text.as_ref().unwrap();

        let total_chars = r.chars().count();
        let total_lines = r.lines().count();
        let avg_line_length = total_chars / total_lines;
        if avg_line_length > 150 {
            return Err("generated, avg line length > 150".to_string());
        }

        let total_spaces = r.chars().filter(|x| x.is_whitespace()).count();
        let spaces_percentage = total_spaces as f32 / total_chars as f32;
        if total_lines >= 5 && spaces_percentage <= 0.05 {
            return Err(format!(
                "generated or compressed, {:.1}% spaces < 5%",
                100.0 * spaces_percentage
            ));
        }

        Ok(())
    }
}
