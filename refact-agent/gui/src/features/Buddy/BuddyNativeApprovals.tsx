import { useRef, useState } from "react";
import { v4 as uuidv4 } from "uuid";
import { Button } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { useIsDocumentVisible } from "../../hooks/useIsDocumentVisible";
import { buddyOperatorIdentity } from "../../services/refact/buddyOperator";
import {
  useGetBuddyNativeApprovalsQuery,
  useRespondBuddyNativeApprovalMutation,
  type ApprovalResponse,
  type NativeApprovalSession,
  type NativeApprovalsSnapshot,
} from "../../services/refact/buddyApprovals";
import styles from "./BuddyNativeApprovals.module.css";

const warningText = {
  native_provider_error:
    "Native model/provider configuration reported a load error. Check provider settings; this does not prove the network is offline.",
  native_privacy_policy_error:
    "Native privacy policy could not be loaded reliably. Native restrictive policy remains authoritative; review privacy settings.",
  native_storage_error:
    "Native chat storage reported a save error. Changes may not survive a restart.",
  native_runtime_error:
    "Native chat reported a runtime error. Open the chat for its diagnostic details.",
  provider_usage_stale:
    "Provider usage is stale. Current usage and cost are not confirmed.",
};
const browserChoices = [
  "actions",
  "console",
  "network",
  "mutations",
  "screenshot",
] as const;
type BrowserChoice = (typeof browserChoices)[number];

function ApprovalSession({
  session,
  snapshot,
  identity,
  disabled,
  refresh,
}: {
  session: NativeApprovalSession;
  snapshot: NativeApprovalsSnapshot;
  identity: string;
  disabled: boolean;
  refresh: () => void;
}) {
  const [respond] = useRespondBuddyNativeApprovalMutation();
  const [selection, setSelection] = useState<
    Record<string, "accept" | "reject">
  >({});
  const [browserSelection, setBrowserSelection] = useState<BrowserChoice[]>([]);
  const [browserConfirmed, setBrowserConfirmed] = useState(false);
  const [sent, setSent] = useState(false);
  const [sending, setSending] = useState(false);
  const [failed, setFailed] = useState(false);
  const lock = useRef(false);
  const blocked = disabled || sending || sent || failed;
  const send = async (response: ApprovalResponse) => {
    if (lock.current || blocked) return;
    lock.current = true;
    setSending(true);
    try {
      await respond({
        endpoint_identity: identity,
        chat_id: session.chat_id,
        project_root: snapshot.project_root,
        engine_identity: snapshot.engine_identity,
        source_version: session.source_version,
        client_request_id: uuidv4(),
        response,
      }).unwrap();
      setSent(true);
    } catch {
      setFailed(true);
    } finally {
      setSending(false);
      lock.current = false;
      refresh();
    }
  };
  return (
    <article
      className={styles.session}
      aria-label={`Native approvals for chat ${session.chat_id}`}
    >
      <h4>Chat {session.chat_id}</h4>
      {session.warnings.map((warning) => (
        <p role="status" key={warning}>
          {warningText[warning]}
        </p>
      ))}
      {session.tools.map((tool) => (
        <fieldset key={tool.tool_call_id} disabled={blocked}>
          <legend>
            {tool.tool_name}: {tool.tool_call_id}
          </legend>
          <pre className={styles.command}>{tool.command}</pre>
          <p>{tool.rule}</p>
          {tool.rule ===
          "Privacy approval required: command read guarded files" ? (
            <p>
              The command already ran and read guarded files. Accept releases
              its guarded output to the model; reject withholds that output.
              This does not undo the command.
            </p>
          ) : (
            <p>
              This responds only to this native confirmation. It does not change
              permission rules or approve other tools.
            </p>
          )}
          {(["accept", "reject"] as const).map((choice) => (
            <label key={choice}>
              <input
                type="radio"
                name={`${session.chat_id}:${tool.tool_call_id}`}
                checked={selection[tool.tool_call_id] === choice}
                onChange={() =>
                  setSelection((previous) => ({
                    ...previous,
                    [tool.tool_call_id]: choice,
                  }))
                }
              />
              {choice === "accept"
                ? "Accept this request"
                : "Reject this request"}
            </label>
          ))}
          <Button
            size="sm"
            disabled={blocked || !selection[tool.tool_call_id]}
            onClick={() =>
              void send({
                kind: "tool",
                tool_call_id: tool.tool_call_id,
                accepted: selection[tool.tool_call_id] === "accept",
              })
            }
          >
            Submit tool decision
          </Button>
        </fieldset>
      ))}
      {session.browser_request_id && (
        <fieldset disabled={blocked}>
          <legend>Browser context for the pending message</legend>
          <p>
            Choose which current browser data to include with your pending
            message. Console, network and screenshots may contain private
            information. Unselected categories are omitted; submitting with none
            selected still sends your message. This is not permission to control
            the browser.
          </p>
          {browserChoices.map((choice) => (
            <label key={choice}>
              <input
                type="checkbox"
                checked={browserSelection.includes(choice)}
                onChange={(event) => {
                  setBrowserConfirmed(false);
                  setBrowserSelection((previous) =>
                    event.target.checked
                      ? [...previous, choice]
                      : previous.filter((item) => item !== choice),
                  );
                }}
              />
              Include {choice}
            </label>
          ))}
          <label>
            <input
              type="checkbox"
              checked={browserConfirmed}
              onChange={(event) => setBrowserConfirmed(event.target.checked)}
            />
            I confirm this context selection and want to send the pending
            message
          </label>
          <Button
            size="sm"
            disabled={blocked || !browserConfirmed}
            onClick={() =>
              void send({
                kind: "browser",
                pending_message_id: session.browser_request_id ?? "",
                include_actions: browserSelection.includes("actions"),
                include_console: browserSelection.includes("console"),
                include_network: browserSelection.includes("network"),
                include_mutations: browserSelection.includes("mutations"),
                include_screenshot: browserSelection.includes("screenshot"),
              })
            }
          >
            Send with selected context
          </Button>
        </fieldset>
      )}
      {sending && <p role="status">Submitting to the native owner…</p>}
      {sent && (
        <p role="status">
          Response queued, not resolved. Waiting for a fresh native snapshot.
        </p>
      )}
      {failed && (
        <p role="alert">
          Response not confirmed. Check the native chat before retrying; no
          success is assumed.
        </p>
      )}
    </article>
  );
}

function Approvals({ identity }: { identity: string }) {
  const visible = useIsDocumentVisible();
  const query = useGetBuddyNativeApprovalsQuery(
    { endpoint_identity: identity },
    {
      pollingInterval: visible ? 5000 : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  const snapshot = query.currentData;
  return (
    <section className={styles.approvals} aria-label="Native approvals">
      <h3>Native approvals and warnings</h3>
      <p>
        Independent of Buddy intelligence, appearance and storage. Coverage is
        limited to at most 200 loaded chats and 100 confirmations per chat;
        unloaded chats and other native permission surfaces are not included.
      </p>
      <Button size="sm" variant="ghost" onClick={() => void query.refetch()}>
        Refresh native approvals
      </Button>
      {query.isFetching && (
        <p role="status">Refreshing native approval state…</p>
      )}
      {Boolean(query.error) && (
        <p role="alert">
          Native connection or snapshot unavailable. Approval controls are
          disabled; reconnect or use the native chat. This does not mean
          requests were resolved.
        </p>
      )}
      {snapshot?.truncated && (
        <p role="status">
          Partial coverage: more native requests may exist. Open the remaining
          chats.
        </p>
      )}
      {!query.error &&
        snapshot?.warnings.map((warning) => (
          <p role="status" key={warning}>
            {warningText[warning]}
          </p>
        ))}
      {snapshot &&
        !query.error &&
        !snapshot.warnings.length &&
        snapshot.sessions.every(
          (session) =>
            !session.tools.length &&
            !session.browser_request_id &&
            !session.warnings.length,
        ) && (
          <p>No approvals or warnings found in the loaded native sessions.</p>
        )}
      {snapshot?.sessions
        .filter(
          (session) =>
            session.tools.length ||
            session.browser_request_id ||
            session.warnings.length,
        )
        .map((session) => (
          <ApprovalSession
            key={JSON.stringify([
              identity,
              snapshot.project_root,
              snapshot.engine_identity,
              session.chat_id,
              session.source_version,
            ])}
            session={session}
            snapshot={snapshot}
            identity={identity}
            disabled={Boolean(query.error) || query.isFetching}
            refresh={() => {
              void query.refetch();
            }}
          />
        ))}
    </section>
  );
}
export function BuddyNativeApprovals() {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <Approvals key={identity} identity={identity} />;
}
