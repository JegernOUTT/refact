use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProvenance {
    pub agent: String,
    pub tier: u8,
    pub confidence: f64,
}

const KNOWN_AGENTS: &[&str] = &[
    "copilot-swe-agent",
    "devin-ai-integration",
    "cursoragent",
    "google-labs-jules",
    "codegen-sh",
    "openhands-ai",
    "sweep-ai",
    "claude",
];

const AGENT_DOMAINS: &[&str] = &[
    "@anthropic.com",
    "@openai.com",
    "@cursor.com",
    "@devin.ai",
    "@sweep.dev",
];

pub fn classify_commit(author: &str, committer: &str, message: &str) -> Option<AgentProvenance> {
    let identities = format!("{} {}", author, committer).to_lowercase();
    let lower_message = message.to_lowercase();
    for agent in KNOWN_AGENTS {
        if identities.contains(agent) {
            return Some(AgentProvenance {
                agent: (*agent).to_string(),
                tier: 1,
                confidence: 0.95,
            });
        }
    }

    if lower_message.contains("generated with claude code") {
        return Some(agent("claude", 2, 0.85));
    }
    if lower_message.contains("co-authored-by: claude") {
        return Some(agent("claude", 2, 0.85));
    }
    if lower_message.contains("opencode") {
        return Some(agent("opencode", 2, 0.85));
    }
    if lower_message.contains("codex") {
        return Some(agent("codex", 2, 0.85));
    }
    if author.trim_end().to_lowercase().ends_with("(aider)") {
        return Some(agent("aider", 2, 0.85));
    }

    for line in lower_message.lines() {
        if line.starts_with("co-authored-by:") && AGENT_DOMAINS.iter().any(|d| line.contains(d)) {
            return Some(agent("agent-domain", 3, 0.7));
        }
    }

    None
}

fn agent(agent: &str, tier: u8, confidence: f64) -> AgentProvenance {
    AgentProvenance {
        agent: agent.to_string(),
        tier,
        confidence,
    }
}
