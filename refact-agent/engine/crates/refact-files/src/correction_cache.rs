use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

struct TrieNode {
    children: HashMap<usize, TrieNode>,
    count: usize,
    is_root: bool,
}

impl TrieNode {
    fn new() -> Self {
        TrieNode {
            children: HashMap::new(),
            count: 0,
            is_root: false,
        }
    }
}

pub struct PathTrie {
    root: TrieNode,
    index_to_component: HashMap<usize, String>,
}

fn shortest_root_path<'a>(path: &PathBuf, root_paths: &'a [PathBuf]) -> Option<&'a PathBuf> {
    root_paths
        .iter()
        .find(|root_path| path.strip_prefix(root_path).is_ok())
}

pub struct ShortPathsIter<'a> {
    trie: &'a PathTrie,
    stack: Vec<(&'a TrieNode, HashSet<usize>, String)>,
}

impl<'a> Iterator for ShortPathsIter<'a> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((node, indices_to_process, _)) = self.stack.last_mut() {
            if node.is_root || node.children.is_empty() {
                let mut path = PathBuf::new();
                for (_, _, component) in self.stack.iter().rev() {
                    if !component.is_empty() {
                        path.push(component);
                    }
                }
                self.stack.pop();
                return Some(path.to_string_lossy().to_string());
            }
            if let Some(index) = indices_to_process.iter().next().cloned() {
                indices_to_process.remove(&index);
                let child = node.children.get(&index).unwrap();
                let mut component = self.trie.index_to_component.get(&index).unwrap().clone();
                if child.is_root {
                    if let Some(last_component) =
                        PathBuf::from(component.clone()).components().last()
                    {
                        component = last_component.as_os_str().to_string_lossy().to_string();
                    }
                    if node.children.len() < 2 {
                        component = String::new();
                    }
                }
                self.stack.push((
                    child,
                    child.children.keys().cloned().collect::<HashSet<usize>>(),
                    component,
                ));
            } else {
                self.stack.pop();
            }
        }
        None
    }
}

impl PathTrie {
    pub fn new() -> Self {
        PathTrie {
            root: TrieNode::new(),
            index_to_component: HashMap::new(),
        }
    }

    pub fn build(paths: &Vec<PathBuf>, root_paths: &Vec<PathBuf>) -> Self {
        let mut root = TrieNode::new();
        let mut component_to_index = HashMap::new();
        let mut index_to_component = HashMap::new();

        let mut sorted_root_paths = root_paths.clone();
        sorted_root_paths.sort_by(|a, b| {
            let component_count_a = a.components().count();
            let component_count_b = b.components().count();
            match component_count_a.cmp(&component_count_b) {
                std::cmp::Ordering::Equal => a.cmp(b),
                other => other,
            }
        });

        let mut components: Vec<std::borrow::Cow<str>> = Vec::new();
        for path in paths.iter() {
            let root_path = shortest_root_path(path, &sorted_root_paths);
            let root_path_components = root_path.map_or(0, |p| p.components().count());
            let root_path_str = root_path.map(|p| p.to_string_lossy());

            components.clear();
            components.extend(path.components().map(|comp| comp.as_os_str().to_string_lossy()));

            let mut node = &mut root;
            for i in (0..components.len()).rev() {
                let is_root = root_path_components == i + 1;
                let component: &str = if is_root {
                    root_path_str.as_deref().unwrap_or_default()
                } else {
                    &components[i]
                };
                let index = if let Some(index) = component_to_index.get(component) {
                    *index
                } else {
                    let index = component_to_index.len();
                    component_to_index.insert(component.to_string(), index);
                    index_to_component.insert(index, component.to_string());
                    index
                };
                node = node.children.entry(index).or_insert_with(TrieNode::new);
                node.count += 1;
                node.is_root = is_root;
                if is_root {
                    break;
                }
            }
        }

        PathTrie {
            root,
            index_to_component,
        }
    }

    fn _search_for_nodes(&self, path: &PathBuf) -> Vec<(&TrieNode, PathBuf)> {
        let mut nodes = vec![];
        let mut components: Vec<String> = path
            .components()
            .map(|comp| comp.as_os_str().to_string_lossy().to_string())
            .collect();

        if components.is_empty() {
            return nodes;
        }

        let mut current = &self.root;
        loop {
            let mut components_prefix = PathBuf::new();
            for component in components.iter() {
                components_prefix.push(component.clone());
            }
            let component = components.pop().unwrap();

            let mut is_next_found = false;
            for (index, child) in current.children.iter() {
                if let Some(child_component) = self.index_to_component.get(index) {
                    if child.is_root {
                        let mut root_path = PathBuf::from(child_component);
                        if !root_path.ends_with(&components_prefix) {
                            continue;
                        }
                        if let Some(last_component) = root_path.components().last() {
                            root_path = PathBuf::from(
                                last_component.as_os_str().to_string_lossy().to_string(),
                            );
                        };
                        if current.children.len() < 2 {
                            root_path = PathBuf::new();
                        }
                        match path.strip_prefix(&components_prefix) {
                            Ok(root_relative_path) => {
                                root_path.push(root_relative_path);
                                nodes.push((child, root_path));
                            }
                            Err(_) => continue,
                        };
                    } else if *child_component == component {
                        is_next_found = true;
                        current = child;
                    }
                }
            }

            if !is_next_found {
                break;
            }

            if components.is_empty() {
                nodes.push((current, path.clone()));
                break;
            }
        }

        nodes
    }

    pub fn find_matches(&self, path: &PathBuf) -> Vec<PathBuf> {
        let mut result = vec![];
        for (root_node, relative_path) in self._search_for_nodes(path) {
            if root_node.is_root {
                result.push(relative_path);
                continue;
            }
            let mut stack = Vec::new();
            stack.push((root_node, vec![]));
            while let Some((node, components)) = stack.pop() {
                if node.children.is_empty() {
                    let mut matched_path = PathBuf::new();
                    for index in components.iter().rev() {
                        let component = self.index_to_component.get(index).unwrap();
                        matched_path.push(component);
                    }
                    matched_path.push(path);
                    result.push(matched_path);
                } else {
                    for (index, child) in &node.children {
                        let mut child_components = components.clone();
                        child_components.push(*index);
                        stack.push((child, child_components));
                    }
                }
            }
        }
        result
    }

    pub fn short_path(&self, path: &PathBuf) -> Option<PathBuf> {
        let nodes = self._search_for_nodes(path);
        if nodes.len() == 1 && nodes[0].0.count == 1 {
            let mut node;
            let mut relative_path;
            (node, relative_path) = nodes[0].clone();
            while !node.is_root && !node.children.is_empty() {
                let index;
                (index, node) = node.children.iter().last().unwrap();

                let mut child_relative_path =
                    PathBuf::from(self.index_to_component.get(index).unwrap().clone());
                if let Some(component) = child_relative_path.components().last() {
                    child_relative_path =
                        PathBuf::from(component.as_os_str().to_string_lossy().to_string());
                }
                if node.children.len() < 2 {
                    child_relative_path = PathBuf::new();
                }

                child_relative_path.push(relative_path.clone());
                relative_path = child_relative_path;
            }
            Some(relative_path)
        } else {
            None
        }
    }

    pub fn short_paths_iter(&self) -> ShortPathsIter<'_> {
        ShortPathsIter {
            trie: self,
            stack: vec![(
                &self.root,
                self.root
                    .children
                    .keys()
                    .cloned()
                    .collect::<HashSet<usize>>(),
                String::new(),
            )],
        }
    }

    pub fn len(&self) -> usize {
        let mut count = 0;
        for (_, child) in &self.root.children {
            count += child.count;
        }
        count
    }
}

pub struct CacheCorrection {
    pub filenames: PathTrie,
    pub directories: PathTrie,
}

impl CacheCorrection {
    pub fn new() -> Self {
        CacheCorrection {
            filenames: PathTrie::new(),
            directories: PathTrie::new(),
        }
    }

    pub fn build(paths: &Vec<PathBuf>, workspace_folders: &Vec<PathBuf>) -> CacheCorrection {
        let mut sorted_paths = paths.clone();
        sorted_paths.sort();

        let filenames = PathTrie::build(&sorted_paths, &workspace_folders);

        let mut directories: Vec<PathBuf> = {
            let mut unique_directories = HashSet::new();
            for p in sorted_paths.iter() {
                if let Some(parent) = p.parent() {
                    unique_directories.insert(parent);
                }
            }
            unique_directories
                .iter()
                .map(|p| PathBuf::from(p))
                .collect()
        };
        directories.sort();

        let directories = PathTrie::build(&directories, &workspace_folders);
        CacheCorrection {
            filenames,
            directories,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn test_match() {
        let paths = vec![
            PathBuf::from("/home/user/project1/file1.ext"),
            PathBuf::from("/home/user/project1/file2.ext"),
            PathBuf::from("/home/user/project2/file1.ext"),
            PathBuf::from("/home/user/project2/project3/file1.ext"),
            PathBuf::from("/home/user/project5/file1.ext"),
        ];

        let workspace_folders = vec![
            PathBuf::from("/home/user/project1"),
            PathBuf::from("/home/user/project2"),
            PathBuf::from("/home/user/project2/project3"),
            PathBuf::from("/home/user/project4"),
        ];

        let trie = PathTrie::build(&paths, &workspace_folders);

        assert_eq!(
            trie.find_matches(&PathBuf::from("project4")).len(),
            0,
            "Invalid number of matches (none)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from("roject2/file1.ext")).len(),
            0,
            "Invalid number of matches (truncated path)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from("file2.ext")).len(),
            1,
            "Invalid number of matches (single filename)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from("user/project2/file1.ext"))
                .len(),
            1,
            "Invalid number of matches (single path)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from("file1.ext")).len(),
            4,
            "Invalid number of matches (multiple)"
        );
    }

    #[test]
    #[cfg(windows)]
    fn test_match() {
        let paths = vec![
            PathBuf::from(r#"C:\\Documents\User 1\project1\file1.ext"#),
            PathBuf::from(r#"C:\\Documents\User 1\project1\file2.ext"#),
            PathBuf::from(r#"C:\\Documents\User 1\project2\file1.ext"#),
            PathBuf::from(r#"C:\\Documents\User 1\project2\project 3\file1.ext"#),
            PathBuf::from(r#"D:\\project 5\file1.ext"#),
        ];

        let workspace_folders = vec![
            PathBuf::from(r#"C:\\Documents\User 1\project1"#),
            PathBuf::from(r#"C:\\Documents\User 1\project2"#),
            PathBuf::from(r#"C:\\Documents\User 1\project2\project 3"#),
            PathBuf::from(r#"C:\\Documents\User 1\project4"#),
        ];

        let trie = PathTrie::build(&paths, &workspace_folders);

        assert_eq!(
            trie.find_matches(&PathBuf::from("project4")).len(),
            0,
            "Invalid number of matches (none)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from(r#"roject2\file1.ext"#))
                .len(),
            0,
            "Invalid number of matches (truncated path)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from("file2.ext")).len(),
            1,
            "Invalid number of matches (single filename)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from(r#"User 1\project2\file1.ext"#))
                .len(),
            1,
            "Invalid number of matches (single path)"
        );
        assert_eq!(
            trie.find_matches(&PathBuf::from("file1.ext")).len(),
            4,
            "Invalid number of matches (multiple)"
        );
    }

    #[test]
    fn test_make_cache() {
        let paths = vec![
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("dir")
                .join("file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo2")
                .join("dir")
                .join("file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo1")
                .join("this_file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo2")
                .join("dir")
                .join("this_file.ext"),
            PathBuf::from("home")
                .join("user")
                .join("repo2")
                .join("dir2"),
        ];

        let workspace_folders = vec![
            PathBuf::from("home").join("user").join("repo1"),
            PathBuf::from("home").join("user").join("repo2"),
        ];

        let cache_correction = CacheCorrection::build(&paths, &workspace_folders);

        let mut cache_shortened_result_vec = cache_correction
            .filenames
            .short_paths_iter()
            .collect::<Vec<_>>();
        let mut expected_result = vec![
            PathBuf::from("repo1")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("repo2")
                .join("dir")
                .join("file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("repo1")
                .join("this_file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("dir")
                .join("this_file.ext")
                .to_string_lossy()
                .to_string(),
            PathBuf::from("dir2").to_string_lossy().to_string(),
        ];

        expected_result.sort();
        cache_shortened_result_vec.sort();

        assert_eq!(
            cache_correction.filenames.len(),
            5,
            "The cache should contain 5 paths"
        );
        assert_eq!(
            cache_shortened_result_vec, expected_result,
            "The result should contain the expected paths, instead it found"
        );
    }

    #[cfg(not(all(target_arch = "aarch64", target_os = "linux")))]
    #[cfg(not(debug_assertions))]
    #[test]
    fn test_make_cache_speed() {
        let workspace_folders = vec![
            PathBuf::from("home").join("user").join("repo1"),
            PathBuf::from("home").join("user").join("repo2"),
            PathBuf::from("home").join("user").join("repo3"),
            PathBuf::from("home").join("user").join("repo4"),
        ];

        let mut paths = Vec::new();
        for i in 0..100000 {
            let path = workspace_folders[i % workspace_folders.len()]
                .join(format!("dir{}", i % 1000))
                .join(format!("dir{}", i / 1000))
                .join(format!("file{}.ext", i));
            paths.push(path);
        }
        let start_time = std::time::Instant::now();

        let cache_correction = CacheCorrection::build(&paths, &workspace_folders);
        let cache_shortened_result_vec = cache_correction
            .filenames
            .short_paths_iter()
            .collect::<Vec<_>>();

        let time_spent = start_time.elapsed();
        println!("make_cache took {} ms", time_spent.as_millis());

        assert_eq!(
            cache_correction.filenames.len(),
            paths.len(),
            "The cache should contain 100000 paths"
        );
        assert_eq!(
            cache_shortened_result_vec.len(),
            paths.len(),
            "The cache shortened should contain 100000 paths"
        );
    }
}
