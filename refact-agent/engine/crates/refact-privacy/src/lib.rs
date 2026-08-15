pub mod destination;
pub mod matching;
pub mod policy;
pub mod record;

pub use destination::{Destination, DestinationId, DestinationKind};
pub use matching::{compile_patterns, CompiledPolicy, PolicyError};
pub use policy::{PrivacyPolicy, ShellBehavior, SubagentPolicy, Zone};
pub use record::{Attribution, FileRecord, PrivacyRecord};
