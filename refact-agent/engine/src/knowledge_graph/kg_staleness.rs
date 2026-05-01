use std::collections::HashSet;
use std::path::PathBuf;
use chrono::{NaiveDate, Utc};

use super::kg_structs::KnowledgeGraph;

#[derive(Debug, Default)]
pub struct StalenessReport {
    pub orphan_file_refs: Vec<(PathBuf, Vec<String>)>,
    pub orphan_docs: Vec<PathBuf>,
    pub stale_by_age: Vec<(PathBuf, i64)>,
    pub past_review: Vec<PathBuf>,
    pub inactive_docs: Vec<PathBuf>,
    pub stale_trajectories: Vec<PathBuf>,
}

impl KnowledgeGraph {
    pub fn check_staleness(
        &self,
        max_age_days: i64,
        trajectory_max_age_days: i64,
    ) -> StalenessReport {
        let mut report = StalenessReport::default();
        let today = Utc::now().date_naive();

        for doc in self.docs.values() {
            let kind = doc.frontmatter.kind_or_default();

            if !doc.frontmatter.is_active() {
                report.inactive_docs.push(doc.path.clone());
                continue;
            }

            if let Some(created) = &doc.frontmatter.created {
                if let Ok(created_date) = NaiveDate::parse_from_str(created, "%Y-%m-%d") {
                    let age_days = (today - created_date).num_days();

                    if kind == "trajectory" && age_days > trajectory_max_age_days {
                        report.stale_trajectories.push(doc.path.clone());
                        continue;
                    }

                    if age_days > max_age_days {
                        report.stale_by_age.push((doc.path.clone(), age_days));
                    }
                }
            }

            if let Some(review_after) = &doc.frontmatter.review_after {
                if let Ok(review_date) = NaiveDate::parse_from_str(review_after, "%Y-%m-%d") {
                    if today > review_date {
                        report.past_review.push(doc.path.clone());
                    }
                }
            }

            let missing_files: Vec<String> = doc
                .frontmatter
                .filenames
                .iter()
                .filter(|f| {
                    self.file_index
                        .get(*f)
                        .and_then(|idx| self.graph.node_weight(*idx))
                        .map(|node| {
                            if let super::kg_structs::KgNode::FileRef { exists, .. } = node {
                                !exists
                            } else {
                                false
                            }
                        })
                        .unwrap_or(true)
                })
                .cloned()
                .collect();

            if !missing_files.is_empty() && doc.frontmatter.kind_or_default() == "code" {
                report
                    .orphan_file_refs
                    .push((doc.path.clone(), missing_files));
            }
        }

        let docs_with_links: HashSet<PathBuf> = self
            .docs
            .values()
            .flat_map(|d| d.frontmatter.links.iter())
            .filter_map(|link| self.docs.get(link))
            .map(|d| d.path.clone())
            .collect();

        for doc in self.docs.values() {
            if doc.frontmatter.tags.is_empty()
                && doc.frontmatter.filenames.is_empty()
                && doc.entities.is_empty()
                && !docs_with_links.contains(&doc.path)
                && doc.frontmatter.kind_or_default() != "trajectory"
            {
                report.orphan_docs.push(doc.path.clone());
            }
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;
    use super::super::kg_structs::{KnowledgeDoc, KnowledgeFrontmatter, KnowledgeGraph};

    fn doc(path: &str, frontmatter: KnowledgeFrontmatter) -> KnowledgeDoc {
        KnowledgeDoc {
            path: PathBuf::from(path),
            frontmatter,
            content: String::new(),
            entities: Vec::new(),
        }
    }

    #[test]
    fn inactive_docs_are_reported_for_removal() {
        let mut graph = KnowledgeGraph::new();
        graph.add_doc(doc(
            "/tmp/deprecated.md",
            KnowledgeFrontmatter {
                id: Some("deprecated".to_string()),
                status: Some("deprecated".to_string()),
                ..Default::default()
            },
        ));
        graph.add_doc(doc(
            "/tmp/archived.md",
            KnowledgeFrontmatter {
                id: Some("archived".to_string()),
                status: Some("archived".to_string()),
                ..Default::default()
            },
        ));

        let report = graph.check_staleness(180, 90);

        assert!(report
            .inactive_docs
            .contains(&PathBuf::from("/tmp/deprecated.md")));
        assert!(report
            .inactive_docs
            .contains(&PathBuf::from("/tmp/archived.md")));
    }

    #[test]
    fn active_docs_past_max_age_are_stale() {
        let old_date = (Utc::now() - Duration::days(181))
            .format("%Y-%m-%d")
            .to_string();
        let mut graph = KnowledgeGraph::new();
        graph.add_doc(doc(
            "/tmp/stale.md",
            KnowledgeFrontmatter {
                id: Some("stale".to_string()),
                status: Some("active".to_string()),
                created: Some(old_date),
                ..Default::default()
            },
        ));

        let report = graph.check_staleness(180, 90);

        assert_eq!(report.stale_by_age.len(), 1);
        assert_eq!(report.stale_by_age[0].0, PathBuf::from("/tmp/stale.md"));
    }
}
