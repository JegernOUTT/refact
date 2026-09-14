export function resumeReceipt(overrides: Record<string, unknown> = {}) {
  return {
    action: "inspect",
    agent_id: "stopped-child",
    child_chat_id: "child-trajectory",
    status: "failed",
    change_seq: 14,
    run_generation: 3,
    step_count: 20,
    cap_stop: {
      key: "max_steps",
      scope: "child",
      limit: 20,
      used: 20,
      resumable: true,
      run_generation: 3,
    },
    can_resume: false,
    can_raise_and_resume: true,
    max_steps: 20,
    settings_subagent_max_steps_ceiling: 1000,
    resume_configuration_available: true,
    original_scope_available: true,
    ...overrides,
  };
}
