# Refact without an IDE

Refact runs everywhere the terminal does — no editor, no plugin, no graphical desktop required. A single `refact` binary gives you the resident daemon, a browser dashboard, a full-screen TUI, workspace panels, and a headless agent CLI. This guide covers the full IDE-free journey from first install to scheduled autonomous runs and remote access.

## Install

**Unix/macOS**

```sh
curl -fsSL https://raw.githubusercontent.com/JegernOUTT/refact/main/install.sh | sh
```

**Windows PowerShell**

```powershell
irm https://raw.githubusercontent.com/JegernOUTT/refact/main/install.ps1 | iex
```

The installer places `refact` on your `PATH`. No dependencies, no runtime, no sudo — just the binary.

Additional packaging channels (Homebrew, Winget, Scoop, npm) are coming — watch the [releases page](https://github.com/JegernOUTT/refact/releases) for availability.

Verify it worked:

```sh
refact version
```

## Your first launch: `refact ui`

Open the dashboard in your default browser:

```sh
refact ui
```

That single command starts or reuses the resident daemon and prints the dashboard URL. The daemon stays warm in the background; subsequent `refact ui` calls open instantly.

To open a specific project:

```sh
refact ui .
refact ui ~/my-project
```

### What the dashboard gives you

The dashboard runs in your browser at `http://127.0.0.1:<port>`. It is a full chat surface — not a read-only status page:

- **Home wizard** — first-run setup walks through provider configuration, BYOK keys, and model selection.
- **Projects fleet** — register projects, switch between them in tabs, and inspect each worker's chats, tools, and status.
- **Doctor** — built-in diagnostics (`refact doctor`) surface as a health check panel inside the dashboard.
- **Agent modes, memory, tasks, settings** — every surface the IDE plugins use is available directly in the browser.

### Browser-only flags

```sh
refact ui --json       # Print the URL as JSON, no browser launch
refact ui --no-open    # Print the URL, skip browser launch
```

These are useful for scripts, Docker containers, or remote machines where you just need the URL to paste into a browser on another device.

## Workspace panels: Files, Git, Terminal

Once a project is open in the dashboard, three workspace panels provide IDE-grade surface area without leaving the browser:

| Panel | What you can do |
|-------|----------------|
| **Files** | Browse, search, and open files in the project tree; preview contents inline with syntax highlighting |
| **Git** | Stage, unstage, diff, and view changed files; create and switch branches; review commit history |
| **Terminal** | Full PTY-backed shell in the project root; persistent sessions survive tab switches |

These panels are backed by the same engine capabilities that power the agent's file, shell, and git tools. They are thin browser surfaces over the local daemon — no file leaves your machine.

> **Capability note:** Workspace panels are available in the dashboard. The TUI (`refact` / `refact tui`) provides a keyboard-first alternative with the same session, tools, and project state.

## Access from other devices

Refact serves its dashboard on `127.0.0.1` by default. To reach it from a phone, tablet, or another machine on your LAN:

### 1. Bind to the network interface

Set the daemon to listen on all interfaces:

```sh
refact daemon --foreground --bind 0.0.0.0
```

Or through configuration — check `~/.config/refact/` for the daemon bind address.

### 2. Enable authentication

When the dashboard is reachable beyond `localhost`, **Basic auth is required.** The daemon generates a token on first start and stores it in `~/.cache/refact/daemon.json`. Access is authenticated through a query parameter appended to the dashboard URL.

Set a static username and password (or use the auto-generated token) — the dashboard login screen handles it.

### 3. QR code access

Open the dashboard, navigate to **Settings → Remote Access**, and scan the QR code from your phone. The QR encodes the authenticated URL so you don't have to type it.

### 4. Firewall note

The daemon port (shown in `refact status` output) must be reachable on your LAN. No inbound internet exposure is required — this is local-network access only.

## Headless CLI

When you don't want a browser or TUI at all, Refact works entirely from the terminal.

### `refact run` — one-shot agent turns

```sh
refact run "Find all unwrap() calls in src/ and suggest replacements"
```

A headless chat turn through the daemon: the engine processes the prompt, streams the response, and exits. Useful for scripts, cron jobs, and CI pipelines.

```sh
refact run --project . --mode explore "Summarize the architecture"
refact run --project . --model deepseek/deepseek-chat "Explain this function"
refact run --project . --approve auto --timeout-secs 300 "Fix all clippy warnings"
```

Key options:

| Flag | Purpose |
|------|---------|
| `--project <path>` | Project root (default: cwd) |
| `--mode agent\|explore` | Chat mode (default: agent) |
| `--model <model>` | Override the default model |
| `--approve deny\|ask\|auto` | Tool approval policy (default: deny) |
| `--timeout-secs <N>` | Timeout in seconds (default: 600) |
| `--json` | Emit final JSON instead of streaming text |

With `--approve deny`, the agent plans and explains but never touches files — safe for exploration. With `--approve auto`, it runs tools autonomously — great for known-safe batch work.

### TUI: `refact` / `refact tui`

```sh
refact
refact tui --project .
```

The full-screen terminal UI mirrors the browser dashboard. It runs the same sessions, tools, and agent modes as the GUI, but in a keyboard-driven terminal interface. Ideal for SSH sessions, tmux workflows, and minimal environments.

### Scheduled autonomous runs: `refact cron`

```sh
refact cron list
refact cron add --every 30m --prompt "Review open TODOs and suggest next steps" --description "todo-review"
refact cron pause <id>
refact cron resume <id>
refact cron rm <id>
```

Jobs can fire on a cron expression, interval, or one-shot schedule with timezone support. Each job delivers its result to the chat, a webhook, a notifier integration, or silently. See `refact cron --help` for the full option set.

### Health checks: `refact doctor`

```sh
refact doctor
```

Diagnoses the daemon setup: binary path, daemon.json validity, daemon reachability, version match, loopback port, worker responsiveness, project roots, and lock file. Exits 0 when everything is healthy, 1 otherwise. Pipe it into a health endpoint:

```sh
refact doctor --json | jq .
```

### Other CLI quick-reference

| Command | Purpose |
|---------|---------|
| `refact ps` | List daemon-managed workers |
| `refact status` | Check daemon health |
| `refact projects open .` | Register or wake a project worker |
| `refact projects list` | List registered projects |
| `refact logs . -f` | Follow a project's worker logs |
| `refact logs --daemon -f` | Follow daemon logs |
| `refact events -f` | Follow daemon events |
| `refact restart --daemon` | Restart the daemon |
| `refact self-update` | Update the installed binary |
| `refact version` | Print version and build info |

## Security notes

Refact runs local-first. Here is what that means for the headless and browser-accessible surfaces:

### Terminal gating

By default, agent-initiated shell commands require user approval. The confirmation popup appears in the dashboard, TUI, or headless `--approve` policy. You control the trust boundary:

- `--approve deny` — the agent can plan and read but never executes. Safest for unattended exploration.
- `--approve ask` — the agent pauses and asks before every tool call. The default for interactive use.
- `--approve auto` — the agent runs tools autonomously. Only use for known-safe batch work and trusted project contexts.

The approval policy is per-run, not global — you can run one script with auto-approve and the next with deny without changing config.

### File-read privacy

Refact respects the privacy configuration at `~/.config/refact/default_privacy.yaml` (or a project-local override). Sensitive files outside the project root are not read by agent tools unless explicitly permitted. Network access is limited to configured providers and integrations — no ambient outbound traffic.

### LAN access security

- The daemon listens on `127.0.0.1` by default and is not reachable from other devices.
- Binding to `0.0.0.0` requires deliberate configuration and Basic auth.
- The QR code in Settings includes the authenticated URL — scan it, don't share it.
- No inbound internet exposure is required or recommended. Use a VPN or SSH tunnel for remote access beyond your LAN.

### Credentials

Provider keys live in `~/.config/refact/providers.d/*.yaml`. They are never bundled with the binary, never sent to a Refact-hosted service, and never appear in logs or trajectories when privacy rules are active.

---

→ Back to [README](https://github.com/JegernOUTT/refact#readme)
→ Full CLI guide: [Installation (CLI)](https://github.com/JegernOUTT/refact/wiki/Installation-CLI)
→ Architecture: [Architecture](https://github.com/JegernOUTT/refact/wiki/Architecture)
