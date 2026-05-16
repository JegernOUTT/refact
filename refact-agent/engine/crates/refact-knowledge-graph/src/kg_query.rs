use std::collections::{BTreeMap, HashSet};

use super::kg_structs::{KnowledgeDoc, KnowledgeGraph};

fn tag_similarity_weight(tag: &str) -> f64 {
    let tag = tag.trim().to_lowercase();
    if tag.starts_with("entity:") || tag.starts_with("symbol:") {
        2.5
    } else if tag.starts_with("component:") || tag.starts_with("workflow:") {
        2.0
    } else if tag.starts_with("domain:")
        || tag.starts_with("tool:")
        || tag.starts_with("verification:")
    {
        1.5
    } else if tag.starts_with("language:")
        || tag.starts_with("framework:")
        || tag.starts_with("state:")
        || tag.starts_with("protocol:")
    {
        1.0
    } else {
        0.5
    }
}

#[derive(Debug, Clone)]
pub struct RelatedDoc {
    pub id: String,
    pub score: f64,
}

impl KnowledgeGraph {
    pub fn find_related(&self, doc_id: &str, max_results: usize) -> Vec<RelatedDoc> {
        let Some(doc) = self.docs.get(doc_id) else {
            return vec![];
        };

        let mut scores: BTreeMap<String, f64> = BTreeMap::new();

        for tag in &doc.frontmatter.tags {
            for related_id in self.docs_with_tag(tag) {
                if related_id != doc_id {
                    *scores.entry(related_id).or_insert(0.0) += tag_similarity_weight(tag);
                }
            }
        }

        for filename in &doc.frontmatter.filenames {
            for related_id in self.docs_referencing_file(filename) {
                if related_id != doc_id {
                    *scores.entry(related_id).or_insert(0.0) += 2.0;
                }
            }
        }

        for entity in &doc.entities {
            for related_id in self.docs_mentioning_entity(entity) {
                if related_id != doc_id {
                    *scores.entry(related_id).or_insert(0.0) += 1.5;
                }
            }
        }

        let mut results: Vec<RelatedDoc> = scores
            .into_iter()
            .filter(|(id, _)| {
                self.docs
                    .get(id)
                    .map(|d| d.frontmatter.is_active())
                    .unwrap_or(false)
            })
            .map(|(id, score)| RelatedDoc { id, score })
            .collect();

        results.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        results.truncate(max_results);
        results
    }

    pub fn expand_search_results(
        &self,
        initial_doc_ids: &[String],
        max_expansion: usize,
    ) -> Vec<String> {
        let mut all_ids: HashSet<String> = initial_doc_ids.iter().cloned().collect();
        let mut expanded: Vec<String> = vec![];

        for doc_id in initial_doc_ids {
            let related = self.find_related(doc_id, max_expansion);
            for rel in related {
                if !all_ids.contains(&rel.id) {
                    all_ids.insert(rel.id.clone());
                    expanded.push(rel.id);
                }
                if expanded.len() >= max_expansion {
                    break;
                }
            }
            if expanded.len() >= max_expansion {
                break;
            }
        }

        expanded
    }

    pub fn find_similar_docs(
        &self,
        tags: &[String],
        filenames: &[String],
        entities: &[String],
    ) -> Vec<(String, f64)> {
        let mut scores: BTreeMap<String, f64> = BTreeMap::new();

        for tag in tags {
            for id in self.docs_with_tag(tag) {
                *scores.entry(id).or_insert(0.0) += tag_similarity_weight(tag);
            }
        }

        for filename in filenames {
            for id in self.docs_referencing_file(filename) {
                *scores.entry(id).or_insert(0.0) += 2.0;
            }
        }

        for entity in entities {
            for id in self.docs_mentioning_entity(entity) {
                *scores.entry(id).or_insert(0.0) += 1.5;
            }
        }

        let mut results: Vec<_> = scores
            .into_iter()
            .filter(|(id, _)| {
                self.docs
                    .get(id)
                    .map(|d| d.frontmatter.is_active())
                    .unwrap_or(false)
            })
            .collect();

        results.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        results
    }

    pub fn get_deprecation_candidates(
        &self,
        new_doc_tags: &[String],
        new_doc_filenames: &[String],
        new_doc_entities: &[String],
        exclude_id: Option<&str>,
    ) -> Vec<&KnowledgeDoc> {
        let similar = self.find_similar_docs(new_doc_tags, new_doc_filenames, new_doc_entities);

        similar
            .into_iter()
            .filter(|(id, score)| *score >= 2.0 && exclude_id.map(|e| e != id).unwrap_or(true))
            .filter_map(|(id, _)| {
                let doc = self.docs.get(&id)?;
                if doc.frontmatter.is_deprecated() || doc.frontmatter.is_archived() {
                    return None;
                }
                if doc.frontmatter.kind_or_default() == "trajectory" {
                    return None;
                }
                Some(doc)
            })
            .take(10)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::kg_structs::{KnowledgeDoc, KnowledgeFrontmatter, KnowledgeGraph};
    use std::path::PathBuf;

    fn doc(id: &str, tags: &[&str], filenames: &[&str], entities: &[&str]) -> KnowledgeDoc {
        KnowledgeDoc {
            path: PathBuf::from(format!("/tmp/{id}.md")),
            frontmatter: KnowledgeFrontmatter {
                id: Some(id.to_string()),
                tags: tags.iter().map(|value| value.to_string()).collect(),
                filenames: filenames.iter().map(|value| value.to_string()).collect(),
                status: Some("active".to_string()),
                ..Default::default()
            },
            content: String::new(),
            entities: entities.iter().map(|value| value.to_string()).collect(),
        }
    }

    #[test]
    fn find_similar_docs_tie_breaks_by_id() {
        let mut graph = KnowledgeGraph::new();
        graph.add_doc(doc("z-doc", &["component:buddy"], &[], &[]));
        graph.add_doc(doc("a-doc", &["component:buddy"], &[], &[]));

        let results = graph.find_similar_docs(&["component:buddy".to_string()], &[], &[]);

        assert_eq!(results[0].0, "a-doc");
        assert_eq!(results[1].0, "z-doc");
    }

    #[test]
    fn find_related_tie_breaks_by_id() {
        let mut graph = KnowledgeGraph::new();
        graph.add_doc(doc("root", &["component:buddy"], &[], &[]));
        graph.add_doc(doc("z-doc", &["component:buddy"], &[], &[]));
        graph.add_doc(doc("a-doc", &["component:buddy"], &[], &[]));

        let related = graph.find_related("root", 10);

        assert_eq!(related[0].id, "a-doc");
        assert_eq!(related[1].id, "z-doc");
    }
}
