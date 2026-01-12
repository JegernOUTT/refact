use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;

fn main() {
    let dir = PathBuf::from("/tmp");
    let target = dir.join("test_target.txt");
    
    let content = "Test content";
    
    let mut tmp_file = NamedTempFile::new_in(&dir).expect("Failed to create temp file");
    tmp_file.write_all(content.as_bytes()).expect("Failed to write");
    tmp_file.flush().expect("Failed to flush");
    
    println!("Temp file created: {:?}", tmp_file.path());
    
    tmp_file.persist(&target).expect("Failed to persist");
    
    println!("File persisted to: {:?}", target);
    
    let read_content = std::fs::read_to_string(&target).expect("Failed to read");
    assert_eq!(read_content, content);
    
    std::fs::remove_file(&target).ok();
    
    println!("Test passed!");
}
