pub mod destination;
pub mod gate;
pub mod matching;
pub mod policy;
pub mod record;
#[cfg(feature = "test-util")]
pub mod testing;

pub use destination::{Destination, DestinationId, DestinationKind};
pub use gate::{clear, records_from_messages, Cleared, PrivacyAudited, Refusal};
pub use matching::{compile_patterns, CompiledPolicy, PolicyError};
pub use policy::{
    load_policy, merge_project, migrate_legacy, parse_policy_yaml, LegacyPrivacyPolicy, PolicyLoad,
    PrivacyPolicy, ShellBehavior, SubagentPolicy, Zone,
};
pub use record::{Attribution, FileRecord, PrivacyRecord};
