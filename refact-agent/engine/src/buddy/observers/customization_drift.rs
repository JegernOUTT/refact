use std::collections::HashMap;
use chrono::{DateTime, Utc};

use crate::buddy::observers::{BuddyObserver, ObserverContext};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyFact, BuddyFactKind};
use crate::app_state::AppState;

pub struct CustomizationDriftObserver;
pub(crate) const MAX_MODE_OVERLAP_CANDIDATES: usize = 100;
const MAX_MODE_PROMPT_CHARS: usize = 4000;

fn cap_prompt(text: &str) -> String {
    text.chars().take(MAX_MODE_PROMPT_CHARS).collect()
}

fn tokenize_for_tf(text: &str) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    for word in text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .map(|w| w.to_string())
    {
        *counts.entry(word).or_insert(0) += 1;
    }
    counts
}

fn cosine_similarity_texts(a: &str, b: &str) -> f32 {
    let tf_a = tokenize_for_tf(a);
    let tf_b = tokenize_for_tf(b);
    let mut vocab: std::collections::HashSet<&String> = tf_a.keys().collect();
    vocab.extend(tf_b.keys());
    let vocab: Vec<&String> = vocab.into_iter().collect();
    let vec_a: Vec<f32> = vocab
        .iter()
        .map(|w| *tf_a.get(*w).unwrap_or(&0) as f32)
        .collect();
    let vec_b: Vec<f32> = vocab
        .iter()
        .map(|w| *tf_b.get(*w).unwrap_or(&0) as f32)
        .collect();
    let dot: f32 = vec_a.iter().zip(vec_b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = vec_a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = vec_b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

fn project_root_hash(path: &std::path::Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    format!("{:x}", h.finish())
}

async fn detect_customization_drift(gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
    let mut facts = vec![];
    let now = ctx.now;

    detect_mode_overlap(gcx.clone(), now, &mut facts).await;
    detect_skill_trigger_weak(gcx.clone(), now, &mut facts).await;
    detect_agents_md_gap(&ctx.project_root, now, &mut facts);

    facts
}

async fn detect_mode_overlap(gcx: AppState, now: DateTime<Utc>, facts: &mut Vec<BuddyFact>) {
    let registry =
        crate::yaml_configs::customization_registry::get_project_registry(gcx.gcx.clone()).await;
    let modes = match registry {
        Some(r) => r.modes,
        None => return,
    };
    let mut candidates: Vec<(String, String)> = modes
        .iter()
        .filter(|(_, m)| m.prompt.len() > 50)
        .map(|(id, m)| (id.clone(), cap_prompt(&m.prompt)))
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.truncate(MAX_MODE_OVERLAP_CANDIDATES);

    let n = candidates.len();
    for i in 0..n {
        for j in (i + 1)..n {
            let sim = cosine_similarity_texts(&candidates[i].1, &candidates[j].1);
            if sim > 0.85 {
                let (a, b) = if candidates[i].0 <= candidates[j].0 {
                    (&candidates[i].0, &candidates[j].0)
                } else {
                    (&candidates[j].0, &candidates[i].0)
                };
                tracing::debug!(
                    "customization_drift: mode_overlap {}~{} sim={:.2}",
                    a,
                    b,
                    sim
                );
                facts.push(BuddyFact {
                    kind: BuddyFactKind::ModePromptOverlap,
                    key: format!("customization:mode_overlap:{}:{}", a, b),
                    source: "customization_drift",
                    payload: serde_json::json!({
                        "mode_id": b,
                        "peer_id": a,
                        "similarity": sim,
                    }),
                    seen_at: now,
                    confidence: 0.8,
                });
            }
        }
    }
}

async fn detect_skill_trigger_weak(gcx: AppState, now: DateTime<Utc>, facts: &mut Vec<BuddyFact>) {
    let ext_dirs = crate::ext::config_dirs::get_ext_dirs(gcx.clone()).await;
    let indices = crate::ext::skills::load_skill_indices(&ext_dirs).await;
    for idx in indices {
        let skill_id = idx.name.clone();
        let full = crate::ext::skills::load_skill_full(&ext_dirs, &skill_id).await;
        let (desc_len, has_context) = match &full {
            Some(f) => (f.index.description.len() as u32, f.context.is_some()),
            None => (idx.description.len() as u32, false),
        };
        if desc_len < 80 || !has_context {
            tracing::debug!(
                "customization_drift: skill_trigger_weak {} desc_len={} has_context={}",
                skill_id,
                desc_len,
                has_context
            );
            facts.push(BuddyFact {
                kind: BuddyFactKind::SkillTriggerWeak,
                key: format!("customization:skill_trigger_weak:{}", skill_id),
                source: "customization_drift",
                payload: serde_json::json!({
                    "skill_id": skill_id,
                    "description_len": desc_len,
                    "has_context": has_context,
                }),
                seen_at: now,
                confidence: 0.7,
            });
        }
    }
}

fn detect_agents_md_gap(
    project_root: &std::path::Path,
    now: DateTime<Utc>,
    facts: &mut Vec<BuddyFact>,
) {
    let agents_md = project_root.join("AGENTS.md");
    let (exists, age_days) = if agents_md.exists() {
        let mtime = agents_md
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs() / 86400)
            .unwrap_or(0);
        (true, mtime as u32)
    } else {
        (false, 0u32)
    };

    if !exists || age_days > 60 {
        let hash = project_root_hash(project_root);
        tracing::debug!(
            "customization_drift: agents_md_gap exists={} age_days={}",
            exists,
            age_days
        );
        facts.push(BuddyFact {
            kind: BuddyFactKind::AgentsMdGapDetected,
            key: format!("customization:agents_md_gap:{}", hash),
            source: "customization_drift",
            payload: serde_json::json!({
                "exists": exists,
                "age_days": age_days,
            }),
            seen_at: now,
            confidence: 0.75,
        });
    }
}

#[async_trait::async_trait]
impl BuddyObserver for CustomizationDriftObserver {
    fn id(&self) -> &'static str {
        "customization_drift"
    }

    fn cadence_seconds(&self) -> u64 {
        1800
    }

    fn requires_setting(&self, settings: &BuddySettings) -> bool {
        settings.observers.customization_drift
            && settings.housekeeping_enabled
            && settings.proactive_enabled
    }

    async fn observe(&self, gcx: AppState, ctx: &ObserverContext) -> Vec<BuddyFact> {
        detect_customization_drift(gcx, ctx).await
    }
}
