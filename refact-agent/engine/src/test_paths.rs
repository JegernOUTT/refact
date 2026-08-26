use std::path::PathBuf;

pub fn abs(relative: &str) -> PathBuf {
    let mut path = if cfg!(windows) {
        PathBuf::from("C:\\")
    } else {
        PathBuf::from("/")
    };
    for part in relative.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path
}

pub fn abs_str(relative: &str) -> String {
    abs(relative).to_string_lossy().into_owned()
}
