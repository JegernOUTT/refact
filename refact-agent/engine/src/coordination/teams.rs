//! Named reusable teams, and roles that bind capability instead of describing
//! it.
//!
//! A role carries a default agent profile, a ceiling on peer authority and how
//! long role-derived peer mail lasts. The ceiling only ever narrows: a role
//! cannot confer authority above its own ceiling, and it cannot confer
//! authority the assigner does not already hold. Placing an agent in a role
//! therefore can never widen what that agent may do.
//!
//! There is one authority path and one roster. A role produces a
//! [`RoleGrantProposal`], which nothing but [`apply_role_grant`] can turn into
//! a grant, and that hands it to the same [`PeerGrantRegistry`] the confirmed
//! `agent_peer_grant` tool writes through. This module never stores an
//! authority of its own. The roster is the seats, and the board card's
//! `team_members` is a projection of them, not a second list.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::tasks::types::{ClaimOutcome, TeamMember};

use super::peer_authority::{PeerAuthority, PeerGrant, PeerGrantRegistry};
use super::types::{Freshness, WorkRow};

/// One role in a team. `authority_ceiling` is the most this role may ever
/// confer, and `None` — the default — means it confers no peer authority at
/// all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleSpec {
    pub name: String,
    pub default_profile: Option<String>,
    pub authority_ceiling: Option<PeerAuthority>,
    pub peer_mail_ttl_secs: u64,
}

impl RoleSpec {
    pub fn new(name: impl Into<String>) -> Self {
        RoleSpec {
            name: name.into(),
            default_profile: None,
            authority_ceiling: None,
            peer_mail_ttl_secs: refact_work_core::coordination_settings()
                .peer_grant_default_ttl_secs,
        }
    }

    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        self.default_profile = Some(profile.into());
        self
    }

    pub fn with_ceiling(mut self, ceiling: PeerAuthority) -> Self {
        self.authority_ceiling = Some(ceiling);
        self
    }

    pub fn with_peer_mail_ttl_secs(mut self, ttl_secs: u64) -> Self {
        self.peer_mail_ttl_secs = ttl_secs;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamSpec {
    pub name: String,
    roles: Vec<RoleSpec>,
}

impl TeamSpec {
    pub fn new(name: impl Into<String>, roles: Vec<RoleSpec>) -> Result<Self, TeamRefusal> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(TeamRefusal::UnnamedTeam);
        }
        let limit = refact_work_core::coordination_settings().team_role_max_count;
        if roles.len() > limit {
            return Err(TeamRefusal::TooManyRoles { limit });
        }
        let mut seen = BTreeSet::new();
        for role in &roles {
            if role.name.trim().is_empty() {
                return Err(TeamRefusal::UnnamedRole);
            }
            if !seen.insert(role.name.clone()) {
                return Err(TeamRefusal::DuplicateRole {
                    role: role.name.clone(),
                });
            }
        }
        Ok(TeamSpec { name, roles })
    }

    pub fn roles(&self) -> &[RoleSpec] {
        &self.roles
    }

    pub fn role(&self, name: &str) -> Option<&RoleSpec> {
        self.roles.iter().find(|role| role.name == name)
    }
}

/// The named teams a project can draw on, bounded by settings.
#[derive(Debug, Clone, Default)]
pub struct TeamCatalog {
    teams: BTreeMap<String, TeamSpec>,
}

impl TeamCatalog {
    pub fn register(&mut self, team: TeamSpec) -> Result<(), TeamRefusal> {
        let limit = refact_work_core::coordination_settings().team_max_count;
        if !self.teams.contains_key(&team.name) && self.teams.len() >= limit {
            return Err(TeamRefusal::TooManyTeams { limit });
        }
        self.teams.insert(team.name.clone(), team);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&TeamSpec> {
        self.teams.get(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.teams.keys().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamRefusal {
    UnnamedTeam,
    UnnamedRole,
    DuplicateRole { role: String },
    TooManyRoles { limit: usize },
    TooManyTeams { limit: usize },
    UnknownTeam { team: String },
}

impl fmt::Display for TeamRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TeamRefusal::UnnamedTeam => write!(f, "a team needs a name"),
            TeamRefusal::UnnamedRole => write!(f, "a role needs a name"),
            TeamRefusal::DuplicateRole { role } => write!(f, "role `{role}` is defined twice"),
            TeamRefusal::TooManyRoles { limit } => {
                write!(f, "a team may define at most {limit} roles")
            }
            TeamRefusal::TooManyTeams { limit } => {
                write!(f, "at most {limit} teams may be registered")
            }
            TeamRefusal::UnknownTeam { team } => write!(f, "team `{team}` is not registered"),
        }
    }
}

/// Who is placing whom in a role, and against which peer.
pub struct RoleAssignment<'a> {
    pub team: &'a str,
    pub project: &'a str,
    /// The principal doing the assigning. Its own authority is the ceiling
    /// above the role's own.
    pub assigner_root: &'a str,
    /// The root that would become the sender of role-derived peer mail.
    pub member_root: &'a str,
    pub recipient_root: &'a str,
}

/// A checked, not-yet-written grant. Only [`propose_role_grant`] builds one and
/// only [`apply_role_grant`] spends one, so a role can reach the registry by no
/// other route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleGrantProposal {
    grant: PeerGrant,
}

impl RoleGrantProposal {
    pub fn authority(&self) -> PeerAuthority {
        self.grant.authority
    }

    pub fn granted_by(&self) -> &str {
        &self.grant.granted_by
    }

    pub fn expires_at_ms(&self) -> u64 {
        self.grant.expires_at_ms
    }

    fn into_grant(self) -> PeerGrant {
        self.grant
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleRefusal {
    RoleConfersNoAuthority {
        role: String,
    },
    AboveRoleCeiling {
        role: String,
        ceiling: PeerAuthority,
        requested: PeerAuthority,
    },
    AssignerLacksAuthority {
        assigner_root: String,
        recipient_root: String,
        requested: PeerAuthority,
    },
    MemberIsAssigner,
    IndistinctIdentities,
}

impl fmt::Display for RoleRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RoleRefusal::RoleConfersNoAuthority { role } => {
                write!(f, "role `{role}` confers no peer authority")
            }
            RoleRefusal::AboveRoleCeiling {
                role,
                ceiling,
                requested,
            } => write!(
                f,
                "role `{role}` ceilings at {ceiling:?} and cannot confer {requested:?}"
            ),
            RoleRefusal::AssignerLacksAuthority {
                assigner_root,
                recipient_root,
                requested,
            } => write!(
                f,
                "`{assigner_root}` does not hold {requested:?} toward `{recipient_root}` and cannot confer it"
            ),
            RoleRefusal::MemberIsAssigner => {
                write!(f, "a role grant cannot target the assigner's own root")
            }
            RoleRefusal::IndistinctIdentities => {
                write!(f, "role grant identities must be distinct and nonempty")
            }
        }
    }
}

/// The non-widening check, and the only place a role turns into authority.
///
/// Three things must hold: the role has a ceiling, the request sits at or below
/// it, and the assigner already holds the requested authority toward the same
/// recipient. The third is what stops delegation from manufacturing power —
/// authority can only be passed along, never created.
pub fn propose_role_grant(
    registry: &PeerGrantRegistry,
    role: &RoleSpec,
    assignment: &RoleAssignment<'_>,
    requested: PeerAuthority,
    now_ms: u64,
) -> Result<RoleGrantProposal, RoleRefusal> {
    if assignment.member_root == assignment.assigner_root {
        return Err(RoleRefusal::MemberIsAssigner);
    }
    if assignment.project.trim().is_empty()
        || assignment.assigner_root.trim().is_empty()
        || assignment.member_root.trim().is_empty()
        || assignment.recipient_root.trim().is_empty()
        || assignment.member_root == assignment.recipient_root
    {
        return Err(RoleRefusal::IndistinctIdentities);
    }
    let ceiling = role
        .authority_ceiling
        .ok_or_else(|| RoleRefusal::RoleConfersNoAuthority {
            role: role.name.clone(),
        })?;
    if !ceiling.allows(requested) {
        return Err(RoleRefusal::AboveRoleCeiling {
            role: role.name.clone(),
            ceiling,
            requested,
        });
    }
    let assigner_grant = registry.get(
        assignment.project,
        assignment.assigner_root,
        assignment.recipient_root,
        now_ms,
    );
    let Some(assigner_grant) = assigner_grant.filter(|grant| grant.authority.allows(requested))
    else {
        return Err(RoleRefusal::AssignerLacksAuthority {
            assigner_root: assignment.assigner_root.to_string(),
            recipient_root: assignment.recipient_root.to_string(),
            requested,
        });
    };
    let max_expiry = now_ms.saturating_add(
        refact_work_core::coordination_settings()
            .peer_grant_max_ttl_secs
            .saturating_mul(1000),
    );
    Ok(RoleGrantProposal {
        grant: PeerGrant {
            project: assignment.project.to_string(),
            sender_root: assignment.member_root.to_string(),
            recipient_root: assignment.recipient_root.to_string(),
            authority: requested,
            granted_by: format!(
                "role:{}/{} assigned by {}",
                assignment.team, role.name, assignment.assigner_root
            ),
            expires_at_ms: now_ms
                .saturating_add(role.peer_mail_ttl_secs.saturating_mul(1000))
                .min(assigner_grant.expires_at_ms)
                .min(max_expiry),
        },
    })
}

/// Spend a proposal through the registry every other grant goes through, so
/// role-derived authority is bounded, expiring and revocable exactly like a
/// user-confirmed one.
pub fn apply_role_grant(
    registry: &PeerGrantRegistry,
    proposal: RoleGrantProposal,
    now_ms: u64,
) -> Result<(), String> {
    registry.grant(proposal.into_grant(), now_ms)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seat {
    pub role: String,
    pub member_chat_id: String,
    pub idempotency_key: String,
    pub branch: Option<String>,
    pub status: Option<String>,
    pub claimed_at_ms: u64,
    pub last_heartbeat_ms: u64,
}

impl Seat {
    fn silent_for_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.last_heartbeat_ms)
    }

    fn to_team_member(&self) -> TeamMember {
        TeamMember {
            role: self.role.clone(),
            agent_chat_id: Some(self.member_chat_id.clone()),
            agent_branch: self.branch.clone(),
            status: self.status.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterRefusal {
    UnknownRole {
        role: String,
    },
    RosterFull {
        limit: usize,
    },
    AlreadyClaimed {
        role: String,
        holder: String,
    },
    NotClaimed {
        role: String,
    },
    NotTheHolder {
        role: String,
        holder: String,
    },
    StillAlive {
        role: String,
        holder: String,
        silent_for_ms: u64,
    },
}

impl fmt::Display for RosterRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RosterRefusal::UnknownRole { role } => {
                write!(f, "this team defines no role `{role}`")
            }
            RosterRefusal::RosterFull { limit } => {
                write!(f, "a roster holds at most {limit} members")
            }
            RosterRefusal::AlreadyClaimed { role, holder } => {
                write!(f, "role `{role}` is held by `{holder}`")
            }
            RosterRefusal::NotClaimed { role } => write!(f, "role `{role}` is unclaimed"),
            RosterRefusal::NotTheHolder { role, holder } => {
                write!(f, "role `{role}` is held by `{holder}`, not by the caller")
            }
            RosterRefusal::StillAlive {
                role,
                holder,
                silent_for_ms,
            } => write!(
                f,
                "`{holder}` still holds `{role}` and last heartbeat was {silent_for_ms}ms ago"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum MissionEvent {
    MemberJoined {
        role: String,
        member: String,
    },
    MemberLeft {
        role: String,
        member: String,
    },
    MemberReclaimed {
        role: String,
        from: String,
        to: String,
    },
    AssignmentMade {
        role: String,
        member: String,
        subject: String,
    },
    OutcomeRecorded {
        role: String,
        member: String,
        outcome: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionEntry {
    pub seq: u64,
    pub at_ms: u64,
    pub event: MissionEvent,
}

/// Append-only within its bound: entries are only ever added, and the bound is
/// honoured by dropping the oldest and counting the drop rather than quietly
/// losing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionLog {
    entries: VecDeque<MissionEntry>,
    capacity: usize,
    next_seq: u64,
    dropped: u64,
}

impl MissionLog {
    pub fn with_capacity(capacity: usize) -> Self {
        MissionLog {
            entries: VecDeque::new(),
            capacity: capacity.max(1),
            next_seq: 1,
            dropped: 0,
        }
    }

    pub fn append(&mut self, at_ms: u64, event: MissionEvent) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.entries.push_back(MissionEntry { seq, at_ms, event });
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
            self.dropped += 1;
        }
        seq
    }

    pub fn entries(&self) -> Vec<MissionEntry> {
        self.entries.iter().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

impl Default for MissionLog {
    fn default() -> Self {
        MissionLog::with_capacity(
            refact_work_core::coordination_settings().team_mission_log_max_entries,
        )
    }
}

/// The seats of one named team: the single roster, from which the board card's
/// `team_members` is rendered.
#[derive(Debug, Clone)]
pub struct TeamRoster {
    team: String,
    roles: BTreeSet<String>,
    seats: BTreeMap<String, Seat>,
    log: MissionLog,
    max_members: usize,
    heartbeat_ttl_ms: u64,
}

impl TeamRoster {
    pub fn for_team(team: &TeamSpec) -> Self {
        let settings = refact_work_core::coordination_settings();
        TeamRoster::with_bounds(
            team,
            settings.team_member_max_count,
            settings.team_mission_log_max_entries,
            settings.team_heartbeat_ttl_ms,
        )
    }

    pub fn with_bounds(
        team: &TeamSpec,
        max_members: usize,
        mission_log_capacity: usize,
        heartbeat_ttl_ms: u64,
    ) -> Self {
        TeamRoster {
            team: team.name.clone(),
            roles: team.roles.iter().map(|role| role.name.clone()).collect(),
            seats: BTreeMap::new(),
            log: MissionLog::with_capacity(mission_log_capacity),
            max_members: max_members.max(1),
            heartbeat_ttl_ms,
        }
    }

    pub fn team(&self) -> &str {
        &self.team
    }

    pub fn mission_log(&self) -> &MissionLog {
        &self.log
    }

    pub fn seat(&self, role: &str) -> Option<&Seat> {
        self.seats.get(role)
    }

    pub fn seats(&self) -> Vec<&Seat> {
        self.seats.values().collect()
    }

    /// Compare-and-set, on the same terms as a board card claim: the same
    /// holder under the same idempotency key already holds it, anyone else
    /// loses, and only a free seat is written.
    pub fn claim(
        &mut self,
        role: &str,
        member_chat_id: &str,
        idempotency_key: &str,
        branch: Option<String>,
        now_ms: u64,
    ) -> Result<ClaimOutcome, RosterRefusal> {
        self.known_role(role)?;
        match self.seats.get(role) {
            Some(existing)
                if existing.member_chat_id == member_chat_id
                    && existing.idempotency_key == idempotency_key =>
            {
                Ok(ClaimOutcome::AlreadyHeld)
            }
            Some(existing) => Err(RosterRefusal::AlreadyClaimed {
                role: role.to_string(),
                holder: existing.member_chat_id.clone(),
            }),
            None => {
                if self.seats.len() >= self.max_members {
                    return Err(RosterRefusal::RosterFull {
                        limit: self.max_members,
                    });
                }
                self.seats.insert(
                    role.to_string(),
                    Seat {
                        role: role.to_string(),
                        member_chat_id: member_chat_id.to_string(),
                        idempotency_key: idempotency_key.to_string(),
                        branch,
                        status: None,
                        claimed_at_ms: now_ms,
                        last_heartbeat_ms: now_ms,
                    },
                );
                self.log.append(
                    now_ms,
                    MissionEvent::MemberJoined {
                        role: role.to_string(),
                        member: member_chat_id.to_string(),
                    },
                );
                Ok(ClaimOutcome::Claimed)
            }
        }
    }

    pub fn heartbeat(
        &mut self,
        role: &str,
        member_chat_id: &str,
        now_ms: u64,
    ) -> Result<(), RosterRefusal> {
        let seat = self.held_seat_mut(role, member_chat_id)?;
        seat.last_heartbeat_ms = now_ms;
        Ok(())
    }

    pub fn release(
        &mut self,
        role: &str,
        member_chat_id: &str,
        now_ms: u64,
    ) -> Result<(), RosterRefusal> {
        self.held_seat_mut(role, member_chat_id)?;
        self.seats.remove(role);
        self.log.append(
            now_ms,
            MissionEvent::MemberLeft {
                role: role.to_string(),
                member: member_chat_id.to_string(),
            },
        );
        Ok(())
    }

    /// Take a seat whose holder stopped heartbeating. A member still inside the
    /// heartbeat window is alive and is never evicted, and the winner's fresh
    /// heartbeat makes the next reclaimer lose the same way — one owner, always.
    pub fn reclaim(
        &mut self,
        role: &str,
        member_chat_id: &str,
        idempotency_key: &str,
        branch: Option<String>,
        now_ms: u64,
    ) -> Result<ClaimOutcome, RosterRefusal> {
        self.known_role(role)?;
        let existing = self
            .seats
            .get(role)
            .ok_or_else(|| RosterRefusal::NotClaimed {
                role: role.to_string(),
            })?;
        if existing.member_chat_id == member_chat_id && existing.idempotency_key == idempotency_key
        {
            return Ok(ClaimOutcome::AlreadyHeld);
        }
        let silent_for_ms = existing.silent_for_ms(now_ms);
        if silent_for_ms <= self.heartbeat_ttl_ms {
            return Err(RosterRefusal::StillAlive {
                role: role.to_string(),
                holder: existing.member_chat_id.clone(),
                silent_for_ms,
            });
        }
        let evicted = existing.member_chat_id.clone();
        self.seats.insert(
            role.to_string(),
            Seat {
                role: role.to_string(),
                member_chat_id: member_chat_id.to_string(),
                idempotency_key: idempotency_key.to_string(),
                branch,
                status: None,
                claimed_at_ms: now_ms,
                last_heartbeat_ms: now_ms,
            },
        );
        self.log.append(
            now_ms,
            MissionEvent::MemberReclaimed {
                role: role.to_string(),
                from: evicted,
                to: member_chat_id.to_string(),
            },
        );
        Ok(ClaimOutcome::Claimed)
    }

    pub fn record_assignment(
        &mut self,
        role: &str,
        member_chat_id: &str,
        subject: &str,
        now_ms: u64,
    ) -> Result<u64, RosterRefusal> {
        self.held_seat_mut(role, member_chat_id)?;
        Ok(self.log.append(
            now_ms,
            MissionEvent::AssignmentMade {
                role: role.to_string(),
                member: member_chat_id.to_string(),
                subject: subject.to_string(),
            },
        ))
    }

    pub fn record_outcome(
        &mut self,
        role: &str,
        member_chat_id: &str,
        outcome: &str,
        now_ms: u64,
    ) -> Result<u64, RosterRefusal> {
        let seat = self.held_seat_mut(role, member_chat_id)?;
        seat.status = Some(outcome.to_string());
        Ok(self.log.append(
            now_ms,
            MissionEvent::OutcomeRecorded {
                role: role.to_string(),
                member: member_chat_id.to_string(),
                outcome: outcome.to_string(),
            },
        ))
    }

    /// Seats whose member has a live source in the coordination catalog. A seat
    /// on its own is a record, not a running agent.
    pub fn live_seats(&self, live_chat_ids: &BTreeSet<String>) -> Vec<&Seat> {
        self.seats
            .values()
            .filter(|seat| live_chat_ids.contains(&seat.member_chat_id))
            .collect()
    }

    /// The board card's view of the roster. Every seat, live or not, so the
    /// stored card keeps saying who holds what; liveness is asked of
    /// [`TeamRoster::live_seats`], never inferred from the card.
    pub fn team_members(&self) -> Vec<TeamMember> {
        self.seats.values().map(Seat::to_team_member).collect()
    }

    pub fn live_team_members(&self, live_chat_ids: &BTreeSet<String>) -> Vec<TeamMember> {
        self.live_seats(live_chat_ids)
            .into_iter()
            .map(Seat::to_team_member)
            .collect()
    }

    fn known_role(&self, role: &str) -> Result<(), RosterRefusal> {
        if self.roles.contains(role) {
            Ok(())
        } else {
            Err(RosterRefusal::UnknownRole {
                role: role.to_string(),
            })
        }
    }

    fn held_seat_mut(
        &mut self,
        role: &str,
        member_chat_id: &str,
    ) -> Result<&mut Seat, RosterRefusal> {
        match self.seats.get_mut(role) {
            None => Err(RosterRefusal::NotClaimed {
                role: role.to_string(),
            }),
            Some(seat) if seat.member_chat_id != member_chat_id => {
                Err(RosterRefusal::NotTheHolder {
                    role: role.to_string(),
                    holder: seat.member_chat_id.clone(),
                })
            }
            Some(seat) => Ok(seat),
        }
    }
}

/// Liveness comes from the coordination catalog and nowhere else: the chats it
/// reports `Live` are the only ones a roster may show as live.
pub fn live_chat_ids(rows: &[WorkRow]) -> BTreeSet<String> {
    rows.iter()
        .filter(|row| row.metadata.freshness == Freshness::Live)
        .filter_map(|row| row.chat_id.clone())
        .collect()
}

#[derive(Clone, Default)]
pub struct TeamRegistry {
    inner: Arc<Mutex<TeamRegistryState>>,
}

#[derive(Default)]
struct TeamRegistryState {
    catalog: TeamCatalog,
    rosters: BTreeMap<(String, String), TeamRoster>,
}

impl TeamRegistry {
    /// Register or replace a team; changing its specification drops all project rosters for it
    /// so seats for removed roles cannot survive.
    pub fn register(&self, team: TeamSpec) -> Result<(), TeamRefusal> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = state
            .catalog
            .get(&team.name)
            .is_some_and(|existing| existing != &team);
        let team_name = team.name.clone();
        state.catalog.register(team)?;
        if changed {
            state
                .rosters
                .retain(|(_, roster_team), _| roster_team != &team_name);
        }
        Ok(())
    }

    pub fn with_roster<T>(
        &self,
        project: &str,
        team: &str,
        f: impl FnOnce(&TeamSpec, &mut TeamRoster) -> T,
    ) -> Result<T, TeamRefusal> {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let spec = state
            .catalog
            .get(team)
            .cloned()
            .ok_or_else(|| TeamRefusal::UnknownTeam {
                team: team.to_string(),
            })?;
        let roster = state
            .rosters
            .entry((project.to_string(), team.to_string()))
            .or_insert_with(|| TeamRoster::for_team(&spec));
        Ok(f(&spec, roster))
    }

    pub fn team_names(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .catalog
            .names()
    }
}

pub const TEAM_CONFIG_KEY: &str = "team";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamConfig {
    roles: Vec<RoleConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleConfig {
    name: String,
    #[serde(default)]
    default_profile: Option<String>,
    #[serde(default)]
    authority_ceiling: Option<String>,
    #[serde(default)]
    peer_mail_ttl_secs: Option<u64>,
}

pub async fn load_team(
    gcx: Arc<crate::global_context::GlobalContext>,
    name: &str,
) -> Result<TeamSpec, String> {
    let unknown = format!(
        "team `{name}` is not defined; define a `team` block in a subagent config before referencing it"
    );
    let registry = crate::yaml_configs::customization_registry::get_project_registry(gcx)
        .await
        .ok_or_else(|| unknown.clone())?;
    let subagent = registry
        .subagents
        .get(name)
        .ok_or_else(|| unknown.clone())?;
    let declaration = subagent
        .extra
        .get(TEAM_CONFIG_KEY)
        .ok_or_else(|| format!("subagent `{name}` has no `{TEAM_CONFIG_KEY}` definition"))?;
    let config: TeamConfig = serde_yaml::from_value(declaration.clone())
        .map_err(|error| format!("team `{name}` is malformed: {error}"))?;
    let roles = config
        .roles
        .into_iter()
        .map(|config| {
            let mut role = RoleSpec::new(config.name);
            if let Some(profile) = config.default_profile {
                role = role.with_profile(profile);
            }
            if let Some(ceiling) = config.authority_ceiling {
                let ceiling = match ceiling.as_str() {
                    "metadata" => PeerAuthority::Metadata,
                    "mail" => PeerAuthority::Mail,
                    "control" => PeerAuthority::Control,
                    _ => {
                        return Err(format!(
                            "team `{name}` has unknown authority ceiling `{ceiling}`"
                        ));
                    }
                };
                role = role.with_ceiling(ceiling);
            }
            if let Some(ttl_secs) = config.peer_mail_ttl_secs {
                let max_ttl_secs =
                    refact_work_core::coordination_settings().peer_grant_max_ttl_secs;
                if !(1..=max_ttl_secs).contains(&ttl_secs) {
                    return Err(format!(
                        "team `{name}` role `{}` has peer_mail_ttl_secs {ttl_secs} outside 1..={max_ttl_secs}",
                        role.name
                    ));
                }
                role = role.with_peer_mail_ttl_secs(ttl_secs);
            }
            Ok(role)
        })
        .collect::<Result<Vec<_>, String>>()?;
    TeamSpec::new(name, roles).map_err(|refusal| refusal.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::types::{
        Disclosure, RootPresence, WorkKindTag, WorkMetadata, WorkRootKind, WorkRootRef,
        WorkStateTag,
    };
    use refact_work_core::WorkKey;

    const PROJECT: &str = "project";
    const TTL: u64 = 60_000;

    fn team() -> TeamSpec {
        TeamSpec::new(
            "ship-it",
            vec![
                RoleSpec::new("implementer")
                    .with_profile("implementer-profile")
                    .with_ceiling(PeerAuthority::Mail)
                    .with_peer_mail_ttl_secs(60),
                RoleSpec::new("reviewer").with_ceiling(PeerAuthority::Metadata),
                RoleSpec::new("observer"),
            ],
        )
        .unwrap()
    }

    fn assignment<'a>(assigner_root: &'a str, member_root: &'a str) -> RoleAssignment<'a> {
        RoleAssignment {
            team: "ship-it",
            project: PROJECT,
            assigner_root,
            member_root,
            recipient_root: "root-lead",
        }
    }

    fn held(authority: PeerAuthority) -> PeerGrant {
        PeerGrant {
            project: PROJECT.to_string(),
            sender_root: "root-assigner".to_string(),
            recipient_root: "root-lead".to_string(),
            authority,
            granted_by: "user".to_string(),
            expires_at_ms: 1_000_000,
        }
    }

    fn roster() -> TeamRoster {
        TeamRoster::with_bounds(&team(), 8, 4, TTL)
    }

    fn row(chat_id: &str, freshness: Freshness) -> WorkRow {
        WorkRow {
            key: WorkKey::trajectory(chat_id),
            metadata: WorkMetadata {
                freshness,
                ..Default::default()
            },
            kind: WorkKindTag::Chat,
            state: WorkStateTag::Running,
            attention: false,
            title: None,
            summary: None,
            root: WorkRootRef {
                id: PROJECT.into(),
                kind: WorkRootKind::Project,
                path: None,
                branch: None,
                project: PROJECT.into(),
                source_path: None,
                presence: RootPresence::Unknown,
            },
            parent_chat_id: None,
            root_chat_id: None,
            chat_id: Some(chat_id.to_string()),
            model: None,
            current_tool: None,
            declared_files: vec![],
            observed_files: vec![],
            queue_size: 0,
            step_count: 0,
            open_questions: 0,
            updated_at_ms: 1,
            stale: false,
            disclosure: Disclosure::Full,
            source_records: vec![],
            provenance_complete: true,
        }
    }

    /// The headline: a role is a ceiling, not a source. It cannot confer what
    /// its assigner does not hold, and it cannot reach the registry except
    /// through the one path every other grant uses.
    #[test]
    fn a_role_cannot_confer_authority_its_assigner_does_not_hold() {
        let registry = PeerGrantRegistry::default();
        let team = team();
        let implementer = team.role("implementer").unwrap();
        let placement = assignment("root-assigner", "root-member");

        let with_no_authority =
            propose_role_grant(&registry, implementer, &placement, PeerAuthority::Mail, 1);
        registry.grant(held(PeerAuthority::Metadata), 1).unwrap();
        let above_what_the_assigner_holds =
            propose_role_grant(&registry, implementer, &placement, PeerAuthority::Mail, 2);
        registry.grant(held(PeerAuthority::Control), 2).unwrap();
        let above_the_role_ceiling = propose_role_grant(
            &registry,
            implementer,
            &placement,
            PeerAuthority::Control,
            3,
        );
        let within_both =
            propose_role_grant(&registry, implementer, &placement, PeerAuthority::Mail, 3).unwrap();

        assert_eq!(
            with_no_authority,
            Err(RoleRefusal::AssignerLacksAuthority {
                assigner_root: "root-assigner".to_string(),
                recipient_root: "root-lead".to_string(),
                requested: PeerAuthority::Mail,
            })
        );
        assert_eq!(
            above_what_the_assigner_holds,
            Err(RoleRefusal::AssignerLacksAuthority {
                assigner_root: "root-assigner".to_string(),
                recipient_root: "root-lead".to_string(),
                requested: PeerAuthority::Mail,
            })
        );
        assert_eq!(
            above_the_role_ceiling,
            Err(RoleRefusal::AboveRoleCeiling {
                role: "implementer".to_string(),
                ceiling: PeerAuthority::Mail,
                requested: PeerAuthority::Control,
            })
        );
        assert_eq!(
            propose_role_grant(
                &registry,
                team.role("observer").unwrap(),
                &placement,
                PeerAuthority::Metadata,
                3
            ),
            Err(RoleRefusal::RoleConfersNoAuthority {
                role: "observer".to_string()
            })
        );

        assert!(!registry.allows(
            PROJECT,
            "root-member",
            "root-lead",
            PeerAuthority::Metadata,
            4
        ));
        assert!(within_both
            .granted_by()
            .contains("role:ship-it/implementer"));
        apply_role_grant(&registry, within_both, 4).unwrap();
        assert!(registry.allows(PROJECT, "root-member", "root-lead", PeerAuthority::Mail, 5));
        assert!(!registry.allows(
            PROJECT,
            "root-member",
            "root-lead",
            PeerAuthority::Control,
            5
        ));
    }

    #[test]
    fn the_authority_ladder_stays_strict_through_role_derived_grants() {
        use PeerAuthority::{Control, Mail, Metadata};
        for (ceiling, assigner_holds, requested, expected) in [
            (Metadata, Control, Metadata, true),
            (Metadata, Control, Mail, false),
            (Metadata, Control, Control, false),
            (Mail, Control, Mail, true),
            (Mail, Control, Control, false),
            (Control, Control, Control, true),
            (Control, Mail, Control, false),
            (Control, Mail, Mail, true),
            (Control, Metadata, Mail, false),
            (Control, Metadata, Metadata, true),
        ] {
            let registry = PeerGrantRegistry::default();
            registry.grant(held(assigner_holds), 1).unwrap();
            let role = RoleSpec::new("role").with_ceiling(ceiling);
            let placement = assignment("root-assigner", "root-member");
            let proposal = propose_role_grant(&registry, &role, &placement, requested, 2);

            assert_eq!(
                proposal.is_ok(),
                expected,
                "ceiling {ceiling:?}, held {assigner_holds:?}, requested {requested:?}"
            );
            if let Ok(proposal) = proposal {
                assert_eq!(proposal.authority(), requested);
                apply_role_grant(&registry, proposal, 2).unwrap();
                for higher in [Metadata, Mail, Control] {
                    assert_eq!(
                        registry.allows(PROJECT, "root-member", "root-lead", higher, 3),
                        requested.allows(higher),
                        "granted {requested:?} must satisfy exactly {higher:?} and below"
                    );
                }
            }
        }
    }

    #[test]
    fn a_seat_claim_is_compare_and_set() {
        let mut roster = roster();

        assert_eq!(
            roster
                .claim("implementer", "chat-a", "key-a", None, 10)
                .unwrap(),
            ClaimOutcome::Claimed
        );
        assert_eq!(
            roster
                .claim("implementer", "chat-a", "key-a", None, 11)
                .unwrap(),
            ClaimOutcome::AlreadyHeld
        );
        assert_eq!(
            roster.claim("implementer", "chat-b", "key-b", None, 12),
            Err(RosterRefusal::AlreadyClaimed {
                role: "implementer".to_string(),
                holder: "chat-a".to_string()
            })
        );
        assert_eq!(
            roster.claim("scribe", "chat-c", "key-c", None, 12),
            Err(RosterRefusal::UnknownRole {
                role: "scribe".to_string()
            })
        );
        assert_eq!(roster.seats().len(), 1);
    }

    #[test]
    fn heartbeat_expiry_allows_reclaim_and_a_live_member_is_not_evicted() {
        let mut roster = roster();
        roster
            .claim("implementer", "chat-a", "key-a", None, 100)
            .unwrap();

        let while_alive = roster.reclaim("implementer", "chat-b", "key-b", None, 100 + TTL);
        roster.heartbeat("implementer", "chat-a", 200).unwrap();
        let after_a_heartbeat = roster.reclaim("implementer", "chat-b", "key-b", None, 200 + TTL);
        let after_silence = roster
            .reclaim("implementer", "chat-b", "key-b", None, 200 + TTL + 1)
            .unwrap();

        assert_eq!(
            while_alive,
            Err(RosterRefusal::StillAlive {
                role: "implementer".to_string(),
                holder: "chat-a".to_string(),
                silent_for_ms: TTL,
            })
        );
        assert!(matches!(
            after_a_heartbeat,
            Err(RosterRefusal::StillAlive { .. })
        ));
        assert_eq!(after_silence, ClaimOutcome::Claimed);
        assert_eq!(
            roster.seat("implementer").unwrap().member_chat_id,
            "chat-b".to_string()
        );
        assert_eq!(
            roster.heartbeat("implementer", "chat-a", 500),
            Err(RosterRefusal::NotTheHolder {
                role: "implementer".to_string(),
                holder: "chat-b".to_string()
            })
        );
    }

    #[test]
    fn two_reclaimers_produce_one_owner() {
        let mut roster = roster();
        roster
            .claim("implementer", "chat-a", "key-a", None, 0)
            .unwrap();
        let at = TTL + 1;

        let first = roster.reclaim("implementer", "chat-b", "key-b", None, at);
        let second = roster.reclaim("implementer", "chat-c", "key-c", None, at);

        assert_eq!(first, Ok(ClaimOutcome::Claimed));
        assert_eq!(
            second,
            Err(RosterRefusal::StillAlive {
                role: "implementer".to_string(),
                holder: "chat-b".to_string(),
                silent_for_ms: 0,
            })
        );
        assert_eq!(roster.seats().len(), 1);
        assert_eq!(
            roster.seat("implementer").unwrap().member_chat_id,
            "chat-b".to_string()
        );
    }

    #[test]
    fn a_member_with_no_live_source_is_not_shown_live() {
        let mut roster = roster();
        roster
            .claim("implementer", "chat-a", "key-a", Some("team/a".into()), 1)
            .unwrap();
        roster
            .claim("reviewer", "chat-b", "key-b", None, 1)
            .unwrap();

        let live = live_chat_ids(&[
            row("chat-a", Freshness::Live),
            row("chat-b", Freshness::Recent),
            row("chat-z", Freshness::Live),
        ]);
        let live_members = roster.live_team_members(&live);

        assert_eq!(
            live,
            BTreeSet::from(["chat-a".to_string(), "chat-z".to_string()])
        );
        assert_eq!(roster.team_members().len(), 2);
        assert_eq!(live_members.len(), 1);
        assert_eq!(live_members[0].role, "implementer");
        assert_eq!(live_members[0].agent_chat_id.as_deref(), Some("chat-a"));
        assert_eq!(live_members[0].agent_branch.as_deref(), Some("team/a"));
        assert!(roster.live_seats(&BTreeSet::new()).is_empty());
    }

    #[test]
    fn the_roster_renders_team_members_without_changing_their_shape() {
        let mut roster = roster();
        roster
            .claim(
                "implementer",
                "chat-implementer",
                "key-a",
                Some("team/implementer".into()),
                1,
            )
            .unwrap();
        roster
            .record_outcome("implementer", "chat-implementer", "doing", 2)
            .unwrap();

        let members = roster.team_members();
        let encoded = serde_json::to_string(&members).unwrap();
        let decoded: Vec<TeamMember> = serde_json::from_str(&encoded).unwrap();

        assert_eq!(
            encoded,
            r#"[{"role":"implementer","agent_chat_id":"chat-implementer","agent_branch":"team/implementer","status":"doing"}]"#
        );
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].role, "implementer");
        assert_eq!(decoded[0].status.as_deref(), Some("doing"));
    }

    #[test]
    fn the_mission_log_is_bounded_and_append_only() {
        let mut roster = roster();
        roster
            .claim("implementer", "chat-a", "key-a", None, 1)
            .unwrap();
        roster
            .record_assignment("implementer", "chat-a", "T-1", 2)
            .unwrap();
        roster
            .record_outcome("implementer", "chat-a", "done", 3)
            .unwrap();
        roster.release("implementer", "chat-a", 4).unwrap();
        roster
            .claim("implementer", "chat-b", "key-b", None, 5)
            .unwrap();

        let entries = roster.mission_log().entries();

        assert_eq!(roster.mission_log().capacity(), 4);
        assert_eq!(entries.len(), 4);
        assert_eq!(roster.mission_log().dropped(), 1);
        assert_eq!(
            entries.iter().map(|entry| entry.seq).collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
        );
        assert_eq!(
            entries[0].event,
            MissionEvent::AssignmentMade {
                role: "implementer".to_string(),
                member: "chat-a".to_string(),
                subject: "T-1".to_string()
            }
        );
        assert_eq!(
            entries[3].event,
            MissionEvent::MemberJoined {
                role: "implementer".to_string(),
                member: "chat-b".to_string()
            }
        );
    }

    #[test]
    fn teams_are_named_reusable_and_bounded() {
        let mut catalog = TeamCatalog::default();
        catalog.register(team()).unwrap();

        assert_eq!(catalog.names(), vec!["ship-it".to_string()]);
        assert_eq!(
            catalog
                .get("ship-it")
                .unwrap()
                .role("implementer")
                .unwrap()
                .default_profile
                .as_deref(),
            Some("implementer-profile")
        );
        assert!(catalog.get("ship-it").unwrap().role("scribe").is_none());
        assert_eq!(
            TeamSpec::new("dup", vec![RoleSpec::new("same"), RoleSpec::new("same")]),
            Err(TeamRefusal::DuplicateRole {
                role: "same".to_string()
            })
        );
        assert_eq!(TeamSpec::new("  ", vec![]), Err(TeamRefusal::UnnamedTeam));
    }
}
