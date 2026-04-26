import React from "react";
import { BuddyCanvas } from "./BuddyCanvas";
import { useBuddyState } from "./hooks/useBuddyState";
import { PALETTES, STAGES, SKILLS } from "./constants";
import type { SignalType, Stage } from "./types";

const SIGNAL_GROUPS: {
  label: string;
  signals: {
    key: SignalType;
    label: string;
    variant: "success" | "error" | "purple" | "warning";
  }[];
}[] = [
  {
    label: "Chat",
    signals: [
      { key: "user_message", label: "💬 Msg", variant: "success" },
      { key: "chat_started", label: "▶ Start", variant: "success" },
      { key: "chat_completed", label: "✅ Done", variant: "success" },
      { key: "chat_error", label: "❌ Error", variant: "error" },
      { key: "streaming", label: "📡 Stream", variant: "success" },
      { key: "generating", label: "⚡ Generate", variant: "success" },
    ],
  },
  {
    label: "Tools",
    signals: [
      { key: "tool_used", label: "🔧 Tool", variant: "success" },
      { key: "tool_failed", label: "💥 Failed", variant: "error" },
      { key: "tool_confirmation", label: "⏸ Confirm", variant: "warning" },
      { key: "edit_applied", label: "📝 Edit", variant: "success" },
      { key: "search_done", label: "🔍 Search", variant: "success" },
      { key: "browser_action", label: "🌐 Browser", variant: "purple" },
    ],
  },
  {
    label: "Background",
    signals: [
      { key: "title_generating", label: "📋 Title", variant: "purple" },
      { key: "commit_msg", label: "📦 Commit", variant: "purple" },
      { key: "memory_extract", label: "🧠 Memory", variant: "purple" },
      { key: "knowledge_update", label: "📚 Knowledge", variant: "purple" },
      { key: "indexing", label: "📂 Index", variant: "purple" },
      { key: "vecdb_building", label: "🧮 VecDB", variant: "purple" },
      { key: "ast_parsing", label: "🌳 AST", variant: "purple" },
      { key: "compression", label: "🗜 Compress", variant: "purple" },
    ],
  },
  {
    label: "Tasks",
    signals: [
      { key: "task_created", label: "📋 New", variant: "success" },
      { key: "task_completed", label: "🎯 Done", variant: "success" },
      { key: "task_failed", label: "📋 Fail", variant: "error" },
      { key: "checkpoint_saved", label: "💾 Save", variant: "success" },
      { key: "skill_learned", label: "⭐ Skill", variant: "success" },
    ],
  },
  {
    label: "System",
    signals: [
      { key: "balance_low", label: "💰 Low$", variant: "warning" },
      { key: "connection_lost", label: "📡 Down", variant: "error" },
      { key: "connection_restored", label: "📡 Up", variant: "success" },
      { key: "git_changes", label: "🔀 Git", variant: "warning" },
      { key: "idle_timeout", label: "😴 Idle", variant: "success" },
    ],
  },
];

const MOOD_COLORS: Record<string, string> = {
  happiness: "#F472B6",
  energy: "#3FB950",
  curiosity: "#58A6FF",
  anxiety: "#F85149",
  boredom: "#8B949E",
  affection: "#C084FC",
};
const PERSONALITY_COLORS: Record<string, string> = {
  playfulness: "#F59E0B",
  confidence: "#60A5FA",
  clinginess: "#4ADE80",
  resilience: "#FB923C",
};

const variantStyle: Record<string, React.CSSProperties> = {
  success: { color: "#3fb950", borderColor: "#3fb950" },
  error: { color: "#f85149", borderColor: "#f85149" },
  purple: { color: "#bc8cff", borderColor: "#bc8cff" },
  warning: { color: "#d29922", borderColor: "#d29922" },
  default: { color: "#c9d1d9", borderColor: "#30363d" },
};

const base: React.CSSProperties = {
  padding: "5px 10px",
  background: "#161b22",
  borderRadius: 5,
  fontSize: 11,
  cursor: "pointer",
  border: "1px solid",
  transition: "opacity .15s",
  whiteSpace: "nowrap",
};

export function BuddyDemo(): React.ReactElement {
  const buddy = useBuddyState();
  const { state } = buddy;

  const stage = STAGES[state.progress.stage] ?? STAGES[0];
  const palette = PALETTES[state.paletteIndex] ?? PALETTES[0];
  const nextStage: Stage | undefined = STAGES[state.progress.stage + 1];
  const xpFill =
    nextStage !== undefined
      ? ((state.progress.xp - stage.xpThreshold) /
          (nextStage.xpThreshold - stage.xpThreshold)) *
        100
      : 100;

  const handleEvent = buddy.handleCanvasEvent;

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "300px 1fr 280px",
        gridTemplateRows: "auto 1fr",
        height: "100vh",
        gap: 1,
        background: "#30363d",
        fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
        color: "#c9d1d9",
      }}
    >
      <div
        style={{
          gridColumn: "1/-1",
          background: "#161b22",
          padding: "14px 24px",
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <div style={{ fontSize: 16, fontWeight: 600 }}>
          {stage.emoji} Byte<span style={{ color: "#58a6ff" }}>bud</span> —{" "}
          {palette.name}
        </div>
        <div
          style={{ display: "flex", gap: 16, fontSize: 12, color: "#8b949e" }}
        >
          <span>
            Stage: <b style={{ color: "#c9d1d9" }}>{stage.name}</b>
          </span>
          <span>
            XP: <b style={{ color: "#c9d1d9" }}>{state.progress.xp}</b>
          </span>
          <span>
            Name: <b style={{ color: "#c9d1d9" }}>{state.name}</b>
          </span>
        </div>
      </div>

      <div style={{ background: "#0d1117", padding: 14, overflowY: "auto" }}>
        <div
          style={{
            fontSize: 10,
            textTransform: "uppercase",
            letterSpacing: 1,
            color: "#8b949e",
            marginBottom: 10,
          }}
        >
          🎮 Signals
        </div>

        {SIGNAL_GROUPS.map((group) => (
          <div key={group.label} style={{ marginBottom: 16 }}>
            <div style={{ fontSize: 11, color: "#8b949e", marginBottom: 5 }}>
              {group.label}
            </div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
              {group.signals.map((s) => (
                <button
                  key={s.key}
                  style={{ ...base, ...variantStyle[s.variant] }}
                  onClick={() => buddy.signal(s.key)}
                >
                  {s.label}
                </button>
              ))}
            </div>
          </div>
        ))}

        <div style={{ marginBottom: 16 }}>
          <div style={{ fontSize: 11, color: "#8b949e", marginBottom: 5 }}>
            Controls
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
            <button
              style={{ ...base, ...variantStyle.default }}
              onClick={() => buddy.addXP(50)}
            >
              +50 XP
            </button>
            <button
              style={{ ...base, ...variantStyle.default }}
              onClick={() => buddy.addXP(500)}
            >
              +500 XP
            </button>
            <button
              style={{ ...base, ...variantStyle.default }}
              onClick={() => buddy.addXP(2000)}
            >
              +2K XP
            </button>
            <button
              style={{ ...base, ...variantStyle.default }}
              onClick={buddy.nextPalette}
            >
              🎨 Palette
            </button>
            <button
              style={{ ...base, ...variantStyle.default }}
              onClick={() =>
                buddy.rename(`Buddy${Math.floor(Math.random() * 99)}`)
              }
            >
              🎲 Rename
            </button>
            <button
              style={{ ...base, ...variantStyle.error }}
              onClick={buddy.reset}
            >
              Reset
            </button>
          </div>
        </div>
      </div>

      <div
        style={{
          background: "#0d1117",
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <div style={{ fontSize: 15, fontWeight: 600, marginBottom: 2 }}>
          {state.name}
        </div>
        <div style={{ fontSize: 11, color: "#8b949e", marginBottom: 8 }}>
          {stage.tagline}
        </div>
        <BuddyCanvas state={state} onEvent={handleEvent} />
        <div style={{ marginTop: 14, width: 200 }}>
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              fontSize: 10,
              color: "#8b949e",
              marginBottom: 3,
            }}
          >
            <span>{state.progress.xp} XP</span>
            <span>
              {nextStage !== undefined
                ? `Next: ${nextStage.xpThreshold}`
                : "MAX"}
            </span>
          </div>
          <div
            style={{
              height: 6,
              background: "#21262d",
              borderRadius: 3,
              overflow: "hidden",
            }}
          >
            <div
              style={{
                height: "100%",
                borderRadius: 3,
                width: `${Math.min(100, xpFill)}%`,
                background: "linear-gradient(90deg,#58a6ff,#bc8cff)",
                transition: "width .5s",
              }}
            />
          </div>
        </div>
        <div
          style={{
            marginTop: 6,
            display: "flex",
            gap: 4,
            alignItems: "center",
            fontSize: 10,
            color: "#8b949e",
          }}
        >
          {`Stage ${state.progress.stage + 1}/7`}
          {STAGES.map((_, i) => (
            <div
              key={i}
              style={{
                width: 7,
                height: 7,
                borderRadius: "50%",
                background:
                  i < state.progress.stage
                    ? "#bc8cff"
                    : i === state.progress.stage
                      ? "#58a6ff"
                      : "#21262d",
              }}
            />
          ))}
        </div>
      </div>

      <div style={{ background: "#0d1117", padding: 14, overflowY: "auto" }}>
        <div
          style={{
            fontSize: 10,
            textTransform: "uppercase",
            letterSpacing: 1,
            color: "#8b949e",
            marginBottom: 10,
          }}
        >
          🧠 Mood
        </div>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 1fr",
            gap: 6,
            marginBottom: 14,
          }}
        >
          {Object.entries(state.mood).map(([k, v]) => {
            const val = v as number;
            return (
              <div
                key={k}
                style={{
                  padding: 6,
                  background: "#161b22",
                  border: "1px solid #30363d",
                  borderRadius: 5,
                  fontSize: 10,
                }}
              >
                <div style={{ color: "#8b949e", marginBottom: 3 }}>
                  {k}{" "}
                  <span style={{ float: "right", color: "#c9d1d9" }}>
                    {Math.round(val)}
                  </span>
                </div>
                <div
                  style={{
                    height: 3,
                    background: "#21262d",
                    borderRadius: 2,
                    overflow: "hidden",
                  }}
                >
                  <div
                    style={{
                      width: `${val}%`,
                      height: "100%",
                      borderRadius: 2,
                      background:
                        (MOOD_COLORS[k] as string | undefined) ?? "#FFF",
                      transition: "width .4s",
                    }}
                  />
                </div>
              </div>
            );
          })}
          {Object.entries(state.personality).map(([k, v]) => {
            const val = v as number;
            return (
              <div
                key={k}
                style={{
                  padding: 6,
                  background: "#161b22",
                  border: "1px solid #30363d",
                  borderRadius: 5,
                  fontSize: 10,
                }}
              >
                <div style={{ color: "#8b949e", marginBottom: 3 }}>
                  {k}{" "}
                  <span style={{ float: "right", color: "#c9d1d9" }}>
                    {Math.round(val)}
                  </span>
                </div>
                <div
                  style={{
                    height: 3,
                    background: "#21262d",
                    borderRadius: 2,
                    overflow: "hidden",
                  }}
                >
                  <div
                    style={{
                      width: `${val}%`,
                      height: "100%",
                      borderRadius: 2,
                      background:
                        (PERSONALITY_COLORS[k] as string | undefined) ?? "#FFF",
                      transition: "width .4s",
                    }}
                  />
                </div>
              </div>
            );
          })}
        </div>

        <div
          style={{
            fontSize: 10,
            textTransform: "uppercase",
            letterSpacing: 1,
            color: "#8b949e",
            marginBottom: 10,
          }}
        >
          ⭐ Skills
        </div>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 5,
            marginBottom: 14,
          }}
        >
          {state.skills.length === 0 && (
            <div style={{ fontSize: 11, color: "#8b949e" }}>
              No skills yet...
            </div>
          )}
          {state.skills.map((id) => {
            const skill = SKILLS.find((s) => s.id === id);
            return skill ? (
              <div
                key={id}
                style={{
                  padding: "6px 8px",
                  background: "#161b22",
                  border: "1px solid #30363d",
                  borderRadius: 5,
                  fontSize: 11,
                  display: "flex",
                  justifyContent: "space-between",
                }}
              >
                <span>
                  {skill.icon} {skill.name}
                </span>
                <span
                  style={{
                    fontSize: 9,
                    color: "#8b949e",
                    background: "#21262d",
                    padding: "1px 5px",
                    borderRadius: 3,
                  }}
                >
                  {skill.xpThreshold}XP
                </span>
              </div>
            ) : null;
          })}
        </div>

        <div
          style={{
            fontSize: 10,
            textTransform: "uppercase",
            letterSpacing: 1,
            color: "#8b949e",
            marginBottom: 10,
          }}
        >
          📜 Log
        </div>
        <div>
          {state.log.slice(0, 20).map((entry, i) => (
            <div
              key={i}
              style={{
                padding: "5px 0",
                borderBottom: "1px solid #30363d",
                fontSize: 10,
                color: "#8b949e",
                display: "flex",
                gap: 6,
              }}
            >
              <span>{entry.icon}</span>
              <span style={{ opacity: 0.5, flexShrink: 0 }}>
                {entry.timestamp}
              </span>
              <span
                style={{
                  flex: 1,
                  color: entry.xpGained ? "#3fb950" : "#8b949e",
                }}
              >
                {entry.message}
                {entry.xpGained ? ` ${entry.xpGained}` : ""}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
