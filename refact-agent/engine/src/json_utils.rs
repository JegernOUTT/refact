use crate::custom_error::MapErrToString;

pub fn extract_json_object<T: for<'de> serde::Deserialize<'de>>(text: &str) -> Result<T, String> {
    let start = text
        .find('{')
        .ok_or_else(|| "No opening brace '{' found".to_string())?;
    let end = text
        .rfind('}')
        .ok_or_else(|| "No closing brace '}' found".to_string())?;

    if end <= start {
        return Err("Closing brace appears before opening brace".to_string());
    }

    serde_json::from_str(&text[start..=end]).map_err_to_string()
}

#[cfg(test)]
mod tests {

    use serde::Deserialize;
}
