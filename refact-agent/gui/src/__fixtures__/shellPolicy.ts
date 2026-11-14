import type { ShellPolicy } from "../services/refact/shellPolicy";

export const STUB_SHELL_POLICY: ShellPolicy = {
  mode: "balanced",
  deny: ["sudo", "doas"],
  ask: ["raw::(){ :|:& };:"],
  allow: [],
  trust_caller_confirmation: true,
  llm_validation: {
    enabled: false,
    model: "",
    authority: "ask_only",
    timeout_secs: 8,
    on_failure: "pass",
    cache_per_chat: true,
  },
  execution: { output_limit_lines: 40 },
  catalogue: [
    {
      id: "redirect.device",
      exec: "*",
      level: "critical",
      reason: "Writes directly to a device node.",
      enabled: true,
    },
    {
      id: "rm.root",
      exec: "rm",
      level: "critical",
      reason: "Removes the filesystem root.",
      enabled: true,
    },
    {
      id: "rm.home",
      exec: "rm",
      level: "high",
      reason: "Recursively removes files under a home directory.",
      enabled: true,
    },
  ],
};
