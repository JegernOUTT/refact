pub use refact_ast::fuzzy_search::fuzzy_search;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::files_in_workspace::retrieve_files_in_workspace_folders;

    async fn get_candidates_from_workspace_files() -> Vec<String> {
        let proj_folders = vec![PathBuf::from(".").canonicalize().unwrap()];
        let proj_folder = &proj_folders[0];

        let mut indexing_everywhere = crate::files_blocklist::IndexingEverywhere::default();
        let (workspace_files, _vcs_folders) = retrieve_files_in_workspace_folders(
            proj_folders.clone(),
            &mut indexing_everywhere,
            false,
            false,
        )
        .await;

        workspace_files
            .iter()
            .filter_map(|path| {
                let relative_path = path
                    .strip_prefix(proj_folder)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();

                Some(relative_path)
            })
            .collect()
    }

    #[tokio::test]
    async fn test_fuzzy_search_finds_frog_py() {
        // Arrange
        let correction_candidate = "frog.p".to_string();
        let top_n = 1;

        let candidates = get_candidates_from_workspace_files().await;

        // Act
        let result = fuzzy_search(&correction_candidate, candidates, top_n, &['/', '\\']);

        // Assert
        let expected_result = vec![PathBuf::from("tests")
            .join("emergency_frog_situation")
            .join("frog.py")
            .to_string_lossy()
            .to_string()];

        assert_eq!(
            result, expected_result,
            "It should find the proper frog.py, found {:?} instead",
            result
        );
    }

    #[tokio::test]
    async fn test_fuzzy_search_path_helps_finding_file() {
        // Arrange
        let correction_candidate = PathBuf::from("emergency_frog_situation")
            .join("wo")
            .to_string_lossy()
            .to_string();
        let top_n = 1;

        let candidates = get_candidates_from_workspace_files().await;

        // Act
        let result = fuzzy_search(&correction_candidate, candidates, top_n, &['/', '\\']);

        // Assert
        let expected_result = vec![PathBuf::from("tests")
            .join("emergency_frog_situation")
            .join("work_day.py")
            .to_string_lossy()
            .to_string()];

        assert_eq!(
            result, expected_result,
            "It should find the proper file (work_day.py), found {:?} instead",
            result
        );
    }

    #[tokio::test]
    async fn test_fuzzy_search_filename_weights_more_than_path() {
        // Arrange
        let correction_candidate = "my_file.ext".to_string();
        let top_n = 2;

        let candidates = vec![
            PathBuf::from("my_library")
                .join("implementation")
                .join("my_file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("my_library")
                .join("my_file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("another_file.ext")
                .to_string_lossy()
                .to_string(),
        ];

        // Act
        let result = fuzzy_search(&correction_candidate, candidates, top_n, &['/', '\\']);

        // Assert
        let expected_result = vec![
            PathBuf::from("my_library")
                .join("my_file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("my_library")
                .join("implementation")
                .join("my_file.ext")
                .to_string_lossy()
                .to_string(),
        ];

        let mut sorted_result = result.clone();
        let mut sorted_expected = expected_result.clone();

        sorted_result.sort();
        sorted_expected.sort();

        assert_eq!(
            sorted_result, sorted_expected,
            "The result should contain the expected paths in any order, found {:?} instead",
            result
        );
    }

    // #[cfg(not(debug_assertions))]
    #[ignore]
    #[test]
    fn test_fuzzy_search_speed() {
        // Arrange
        let workspace_paths = vec![
            PathBuf::from("home").join("user").join("repo1"),
            PathBuf::from("home").join("user").join("repo2"),
            PathBuf::from("home").join("user").join("repo3"),
            PathBuf::from("home").join("user").join("repo4"),
        ];

        let mut paths = Vec::new();
        for i in 0..100000 {
            let path = workspace_paths[i % workspace_paths.len()]
                .join(format!("dir{}", i % 1000))
                .join(format!("dir{}", i / 1000))
                .join(format!("file{}.ext", i));
            paths.push(path);
        }
        let start_time = std::time::Instant::now();
        let paths_str = paths
            .iter()
            .map(|x| x.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let correction_candidate = PathBuf::from("file100000")
            .join("dir1000")
            .join("file100000.ext")
            .to_string_lossy()
            .to_string();

        // Act
        let results = fuzzy_search(&correction_candidate, paths_str, 10, &['/', '\\']);

        // Assert
        let time_spent = start_time.elapsed();
        println!("fuzzy_search took {} ms", time_spent.as_millis());
        assert!(
            time_spent.as_millis() < 750,
            "fuzzy_search took {} ms",
            time_spent.as_millis()
        );

        assert_eq!(results.len(), 10, "The result should contain 10 paths");
        println!("{:?}", results);
    }
}
