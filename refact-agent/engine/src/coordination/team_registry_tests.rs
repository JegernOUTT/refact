use crate::coordination::teams::{RoleSpec, TeamRegistry, TeamSpec};
use crate::tasks::types::ClaimOutcome;

fn team() -> TeamSpec {
    TeamSpec::new("ship-it", vec![RoleSpec::new("implementer")]).unwrap()
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
