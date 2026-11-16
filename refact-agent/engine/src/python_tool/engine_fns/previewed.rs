//! Plans whose payload is the call's own arguments: the preview records the state it showed,
//! and the handler refuses when that state moved before the person answered.

use serde_json::Value;

use super::{EngineCtx, EngineError, Plan};

/// The plan of a preview that showed `state`; the handler applies the call's arguments.
pub fn plan(state: Value) -> Plan {
    Plan::new(state, ())
}

/// Called by the handler with the state it reads now; fails when it differs from the approved plan.
pub fn take(ctx: &EngineCtx, function: &str, current: &Value) -> Result<(), EngineError> {
    ctx.take_plan::<()>(function, current)
}
