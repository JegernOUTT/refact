#[cfg(test)]
mod background_tasks_tests {
    use tokio::task::JoinHandle;

    struct BackgroundTasksHolder {
        tasks: Vec<JoinHandle<()>>,
    }

    impl BackgroundTasksHolder {
        fn new(tasks: Vec<JoinHandle<()>>) -> Self {
            BackgroundTasksHolder { tasks }
        }

        async fn abort(&mut self) {
            for task in self.tasks.iter_mut() {
                task.abort();
                let _ = task.await;
            }
            self.tasks.clear();
        }

        fn len(&self) -> usize {
            self.tasks.len()
        }
    }
}

#[cfg(test)]
mod task_types_tests {

    #[derive(Debug, Clone)]
    struct BoardCard {
        id: String,
        title: String,
        column: String,
        depends_on: Vec<String>,
    }

    #[derive(Debug, Clone)]
    struct BoardColumn {
        id: String,
        title: String,
    }

    #[derive(Debug, Clone)]
    struct TaskBoard {
        schema_version: u32,
        rev: u64,
        columns: Vec<BoardColumn>,
        cards: Vec<BoardCard>,
    }

    impl Default for TaskBoard {
        fn default() -> Self {
            Self {
                schema_version: 1,
                rev: 0,
                columns: vec![
                    BoardColumn {
                        id: "planned".into(),
                        title: "Planned".into(),
                    },
                    BoardColumn {
                        id: "doing".into(),
                        title: "Doing".into(),
                    },
                    BoardColumn {
                        id: "done".into(),
                        title: "Done".into(),
                    },
                    BoardColumn {
                        id: "failed".into(),
                        title: "Failed".into(),
                    },
                    BoardColumn {
                        id: "regressed".into(),
                        title: "Regressed".into(),
                    },
                ],
                cards: vec![],
            }
        }
    }

    impl TaskBoard {}
}

#[cfg(test)]
mod trajectory_memos_tests {}

#[cfg(test)]
mod error_handling_tests {}
