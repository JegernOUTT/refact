import { humanizeIdentifier } from "../../utils/displayNames";
import { useState } from "react";
import { Badge, Button } from "../../components/ui";
import {
  useGetEnginesQuery,
  useGetLiveChatsQuery,
  type LiveChat,
} from "../../services/refact/daemon";
import { useObservationPolling } from "./useObservationPolling";
import styles from "./Console.module.css";

type Filter = "all" | "working" | "waiting" | "paused" | "idle";
function chatCategory(chat: LiveChat): Filter | "other" {
  if (chat.paused || chat.session_state === "paused") return "paused";
  if (["generating", "executing_tools"].includes(chat.session_state))
    return "working";
  if (["waiting_ide", "waiting_user_input"].includes(chat.session_state))
    return "waiting";
  if (chat.session_state === "idle") return "idle";
  return "other";
}
export function ActiveChatsPage() {
  const polling = useObservationPolling();
  const query = useGetLiveChatsQuery(undefined, polling);
  const engines = useGetEnginesQuery(undefined, polling);
  const [filter, setFilter] = useState<Filter>("all");
  return (
    <section className={styles.page}>
      <header>
        <h2>Active chats</h2>
        <p>
          Across engines · loaded sessions only, not full saved history.
          Read-only observation does not restore chats or wake engines.
        </p>
      </header>
      <div className={styles.filters} aria-label="Chat status filters">
        {(["all", "working", "waiting", "paused", "idle"] as const).map(
          (value) => (
            <Button
              key={value}
              variant={filter === value ? "soft" : "ghost"}
              aria-pressed={filter === value}
              onClick={() => setFilter(value)}
            >
              {value[0].toUpperCase() + value.slice(1)}
            </Button>
          ),
        )}
      </div>
      {query.isLoading && <p>Loading loaded sessions…</p>}
      {query.isError && (
        <p role="alert">
          Live chat observation unavailable.{" "}
          {query.data
            ? "Showing stale observations, not current activity."
            : "Coverage unknown."}
        </p>
      )}
      {query.data?.engines.map((observation) => {
        const engine = engines.data?.engines.find(
          (row) => row.id === observation.engine_id,
        );
        const chats = observation.chats.filter(
          (chat) => filter === "all" || chatCategory(chat) === filter,
        );
        return (
          <section key={observation.engine_id}>
            <div className={styles.toolbar}>
              <h3>{engine?.label ?? observation.engine_id}</h3>
              <Badge
                tone={observation.status === "available" ? "muted" : "warning"}
              >
                {observation.status === "available"
                  ? "Loaded sessions"
                  : humanizeIdentifier(observation.status)}
              </Badge>
            </div>
            <p className={styles.path}>
              {engine?.root ?? observation.engine_id}
            </p>
            <p>
              Observed:{" "}
              {observation.observed_at_ms
                ? new Date(observation.observed_at_ms).toLocaleTimeString()
                : "Unknown"}
              {query.isError ? " · stale" : ""}
            </p>
            {observation.status !== "available" && (
              <p>
                {observation.error ??
                  "Current sessions unknown. This is not a zero-chat result."}
              </p>
            )}
            {chats.map((chat) => (
              <article
                key={`${observation.engine_id}:${chat.id}`}
                className={styles.row}
              >
                <div className={styles.identity}>
                  <strong>{chat.title || "Untitled chat"}</strong>
                  <span className={styles.path}>
                    {chat.project_root ?? "Project unknown"}
                  </span>
                  <span className={styles.path}>{chat.id}</span>
                </div>
                <div className={styles.identity}>
                  <Badge
                    tone={
                      chatCategory(chat) === "working" ? "success" : "muted"
                    }
                  >
                    {chat.paused
                      ? "Paused"
                      : humanizeIdentifier(chat.session_state)}
                  </Badge>
                  <span className={styles.muted}>
                    {humanizeIdentifier(chat.mode)} ·{" "}
                    {humanizeIdentifier(chat.kind)}
                  </span>
                </div>
                <span className={styles.muted}>Observe-only</span>
              </article>
            ))}
            {observation.status === "available" && chats.length === 0 && (
              <p>No loaded sessions match this filter.</p>
            )}
          </section>
        );
      })}
      {query.data?.engines.length === 0 && (
        <p>No engines observed. Coverage is limited to connected engines.</p>
      )}
    </section>
  );
}
