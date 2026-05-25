use chrono::Utc;

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, REFACT_SELF_CRITIC_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct RefactSelfCriticJob;

const COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const PRIORITY: u32 = 24;

fn build_self_critic_spec(ctx: &BuddyJobContext) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(REFACT_SELF_CRITIC_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let evidence = format!(
        "date={}\nproject_root={}",
        Utc::now().date_naive(),
        project_root
    );
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Run a daily self-critique pass on Refact prompts and recent trajectories.",
        evidence,
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

#[async_trait::async_trait]
impl BuddyJob for RefactSelfCriticJob {
    fn id(&self) -> &str {
        REFACT_SELF_CRITIC_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, _gcx: AppState, _ctx: &BuddyJobContext) -> bool {
        true
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        execute_autonomous_spec(gcx, &ctx, build_self_critic_spec(&ctx)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::buddy::autonomous_workflows::{
        AUTONOMOUS_BUDDY_WORKFLOWS, REFACT_COMPILE_SNIFFER_WORKFLOW_ID,
    };
    use crate::buddy::conversation_ledger::workflow_id_to_mapping;
    use crate::buddy::settings::BuddySettings;
    use crate::buddy::types::{BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse};

    fn test_context(project_root: &Path) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: project_root.to_path_buf(),
            job_state: BuddyJobState::default(),
            workflow_summaries: vec![],
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: BuddyPetState::default(),
            active_quest: None,
            settings: BuddySettings::default(),
            pulse: BuddyPulse::default(),
            facts: vec![],
        }
    }

    #[tokio::test]
    async fn refact_self_critic_runs_on_24h_cooldown() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let ctx = test_context(dir.path());
        let job = RefactSelfCriticJob;

        assert_eq!(job.cooldown_seconds(), 24 * 60 * 60);
        assert!(job.should_run(gcx, &ctx).await);

        let spec = build_self_critic_spec(&ctx);
        assert_eq!(spec.workflow_id, REFACT_SELF_CRITIC_WORKFLOW_ID);
        assert_eq!(spec.project_root, dir.path().to_string_lossy().to_string());
    }

    #[test]
    fn both_workflows_in_autonomous_workflows_metadata() {
        let ids = AUTONOMOUS_BUDDY_WORKFLOWS
            .iter()
            .map(|meta| meta.id)
            .collect::<Vec<_>>();

        assert!(ids.contains(&REFACT_SELF_CRITIC_WORKFLOW_ID));
        assert!(ids.contains(&REFACT_COMPILE_SNIFFER_WORKFLOW_ID));

        let self_critic = workflow_id_to_mapping(REFACT_SELF_CRITIC_WORKFLOW_ID);
        assert_eq!(self_critic.kind, "system");
        assert_eq!(self_critic.badge, Some("Self-Critic"));

        let compile_sniffer = workflow_id_to_mapping(REFACT_COMPILE_SNIFFER_WORKFLOW_ID);
        assert_eq!(compile_sniffer.kind, "system");
        assert_eq!(compile_sniffer.badge, Some("Compile Sniffer"));
    }
}
