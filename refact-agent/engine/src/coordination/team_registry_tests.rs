use std::collections::BTreeSet;

use crate::coordination::teams::{RoleSpec, TeamRefusal, TeamRegistry, TeamSpec};
use crate::tasks::types::ClaimOutcome;

fn team() -> TeamSpec {
    TeamSpec::new("ship-it", vec![RoleSpec::new("implementer")]).unwrap()
}

#[test]
fn registered_team_roster_persists_across_calls() {
    let registry = TeamRegistry::default();
    registry.register(team()).unwrap();

    registry
        .with_roster("project", "ship-it", |_, roster| {
            assert_eq!(
                roster.claim("implementer", "chat-a", "key-a", None, 10),
                Ok(ClaimOutcome::Claimed)
            );
        })
        .unwrap();
    registry
        .with_roster("project", "ship-it", |_, roster| {
            let members = roster.live_team_members(&BTreeSet::from(["chat-a".to_string()]));
            assert_eq!(members.len(), 1);
            assert_eq!(members[0].agent_chat_id.as_deref(), Some("chat-a"));
        })
        .unwrap();
}

#[test]
fn unknown_team_is_refused() {
    let registry = TeamRegistry::default();

    assert_eq!(
        registry.with_roster("project", "missing", |_, _| ()),
        Err(TeamRefusal::UnknownTeam {
            team: "missing".to_string()
        })
    );
}

#[test]
fn re_registering_a_changed_team_drops_stale_rosters() {
    let registry = TeamRegistry::default();
    registry.register(team()).unwrap();
    registry
        .with_roster("project", "ship-it", |_, roster| {
            roster
                .claim("implementer", "chat-a", "key-a", None, 10)
                .unwrap();
        })
        .unwrap();
    registry
        .register(TeamSpec::new("ship-it", vec![RoleSpec::new("reviewer")]).unwrap())
        .unwrap();

    registry
        .with_roster("project", "ship-it", |_, roster| {
            assert!(roster.seats().is_empty());
            assert!(roster
                .claim("implementer", "chat-b", "key-b", None, 20)
                .is_err());
            assert_eq!(
                roster.claim("reviewer", "chat-b", "key-b", None, 20),
                Ok(ClaimOutcome::Claimed)
            );
        })
        .unwrap();
}

#[tokio::test]
async fn app_state_and_gcx_share_one_registry() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    gcx.teams.register(team()).unwrap();
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;

    app.teams
        .with_roster("project", "ship-it", |_, roster| {
            roster
                .claim("implementer", "chat-a", "key-a", None, 10)
                .unwrap();
        })
        .unwrap();
    gcx.teams
        .with_roster("project", "ship-it", |_, roster| {
            assert_eq!(roster.seat("implementer").unwrap().member_chat_id, "chat-a");
        })
        .unwrap();
}
