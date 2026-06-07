## Local-first & endlessly customizable — zero cloud
Refact runs where your code already lives: on your machine, pointed only at the models, runtimes, and integrations you choose. No bundled cloud is required, no account is mandatory, and local/BYOK usage keeps the whole AI stack inspectable, forkable, and yours.

- The engine is a local `refact-lsp` process; network traffic is limited to configured providers and integrations, from hosted APIs to local Ollama/LM Studio-style runtimes.
- Project state is plain project state: trajectories, knowledge, tasks, integration settings, schedules, and VecDB indexes live under `<project>/.refact/`.
- User-level knobs stay local too: privacy rules and provider definitions live under `~/.config/refact/`, while caches, logs, telemetry artifacts, shadow repos, and integration runtime state live under `~/.cache/refact/`.
- Bring your own keys and keep credentials on your machine; BYOK provider settings can target cloud APIs or fully offline local inference with no Refact-hosted control plane in the loop.
- Agent modes, subagents, toolbox/slash commands, code-lens prompts, system prompts, and provider defaults are YAML-configurable from `refact-agent/engine/yaml_configs/defaults/` and its packaged defaults.
- Tool behavior is policy, not magic: tune tool parameters, privacy filters, and confirmation rules so reads, edits, shells, integrations, and autonomous actions match your trust boundary.
- The payoff is ownership: audit every prompt, fork every workflow, swap every model, and shape the assistant around your repo instead of renting a sealed black box.

→ Deep dive: [Privacy](https://github.com/JegernOUTT/refact/wiki/Privacy), [Agent Modes](https://github.com/JegernOUTT/refact/wiki/Agent-Modes)
