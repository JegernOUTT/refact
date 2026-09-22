import { useState } from "react";
import { Button } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import { useGetBuddyNativeWorkQuery } from "../../services/refact/buddyNativeWork";
import { humanizeIdentifier } from "../../utils/displayNames";
import styles from "./BuddyNativeDecisions.module.css";

function NativeWork({ identity }: { identity: string }) {
  const [open, setOpen] = useState(false);
  const [after, setAfter] = useState<string>();
  const query = useGetBuddyNativeWorkQuery(
    { endpoint_identity: identity, after },
    {
      skip: !open,
      pollingInterval: 5000,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
    },
  );
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>Live chats, goals and workers</summary>
      <section className={styles.decisions} aria-label="Native work status">
        <p>
          Live native state for loaded chats only, not a second goal ledger.
          This view does not grant Buddy supervision authority.
        </p>
        <Button
          size="sm"
          variant="ghost"
          disabled={!open || query.isFetching}
          onClick={() => void query.refetch()}
        >
          Refresh native work
        </Button>
        {query.isFetching && <p role="status">Loading native work…</p>}
        {query.error ? (
          <p role="alert">
            Native work is unavailable. Use the existing chat and goal controls;
            no work is reported as resumed.
          </p>
        ) : (
          query.currentData && (
            <>
              {query.currentData.sessions.length === 0 && (
                <p>No loaded sessions on this page.</p>
              )}
              {query.currentData.sessions.map((row) => (
                <article key={row.chat_id} className={styles.question}>
                  <h3>{row.title || "Untitled chat"}</h3>
                  <p>
                    {humanizeIdentifier(row.state)} ·{" "}
                    {row.model || "No model selected"}
                  </p>
                  {row.persistence_pending && (
                    <p>
                      Live changes are awaiting persistence. This is not a
                      restart-safe completion acknowledgment.
                    </p>
                  )}
                  {row.error && <p role="alert">{row.error}</p>}
                  {row.pending_questions > 0 && (
                    <p>
                      {row.pending_questions} question sets need a response.
                    </p>
                  )}
                  {row.approval_count > 0 && (
                    <p>
                      {row.approval_count} tool or privacy approvals await the
                      native chat controls.
                    </p>
                  )}
                  {row.browser_decision_pending && (
                    <p>
                      A browser-context choice awaits the native chat controls.
                    </p>
                  )}
                  {row.goal && (
                    <section aria-label="Native goal">
                      <h4>Goal · {humanizeIdentifier(row.goal.status)}</h4>
                      <p>{row.goal.content}</p>
                      <p>
                        {row.goal.active
                          ? "This chat owns the goal."
                          : "This chat does not own goal pursuit."}
                      </p>
                      <p>
                        Turns: {row.goal.turns_used}
                        {row.goal.max_turns === null
                          ? " (no turn limit)"
                          : ` / ${row.goal.max_turns}`}{" "}
                        · Tokens: {row.goal.tokens_used}
                        {row.goal.max_tokens === null
                          ? " (no token limit)"
                          : ` / ${row.goal.max_tokens}`}
                      </p>
                      <p>
                        {row.goal.latest_verdict
                          ? `Latest recorded verifier verdict: ${row.goal.latest_verdict}`
                          : "No verifier verdict recorded. Worker completion is not verification."}
                      </p>
                      {row.goal.verification_blocked && (
                        <p>
                          Verification is blocked; no completion proof is
                          claimed.
                        </p>
                      )}
                      {row.goal.gaps.length > 0 && (
                        <ul>
                          {row.goal.gaps.map((gap, index) => (
                            <li key={index}>{gap}</li>
                          ))}
                        </ul>
                      )}
                      {row.goal.truncated && (
                        <p>
                          Goal evidence is shortened here; full criteria and
                          history remain in the native goal.
                        </p>
                      )}
                    </section>
                  )}
                  {row.agents.length > 0 && (
                    <details>
                      <summary>Workers</summary>
                      <ul>
                        {row.agents.map((agent) => (
                          <li key={agent.agent_id}>
                            <strong>{agent.title || "Worker"}</strong> ·{" "}
                            {humanizeIdentifier(agent.status)} ·{" "}
                            {agent.model || "Model not reported"}
                            {agent.progress && <p>{agent.progress}</p>}
                          </li>
                        ))}
                      </ul>
                      {row.agents_truncated && (
                        <p>Only the first twenty workers are shown.</p>
                      )}
                    </details>
                  )}
                  <p>Native chat identity: {row.chat_id}</p>
                </article>
              ))}
              {query.currentData.next_cursor !== null && (
                <Button
                  size="sm"
                  disabled={query.isFetching}
                  onClick={() =>
                    setAfter(query.currentData?.next_cursor ?? undefined)
                  }
                >
                  Next loaded chats
                </Button>
              )}
            </>
          )
        )}
        {after !== undefined && (
          <Button size="sm" variant="ghost" onClick={() => setAfter(undefined)}>
            First loaded chats
          </Button>
        )}
      </section>
    </details>
  );
}
export function BuddyNativeWork() {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <NativeWork key={identity} identity={identity} />;
}
