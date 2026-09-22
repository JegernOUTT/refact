import { useEffect, useRef, useState } from "react";
import { v4 as uuidv4 } from "uuid";
import { ShieldAlert, ShieldCheck } from "lucide-react";
import { Badge, Button, Icon, Surface, Text } from "../../components/ui";
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
import { BuddySectionHeader } from "./BuddySectionHeader";
import styles from "./BuddyNativeApprovals.module.css";

const APPROVAL_POLLING_INTERVAL_MS = 5000;

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
    <Surface
      as="article"
      variant="surface-1"
      radius="control"
      className={styles.session}
      aria-label={`Native approvals for chat ${session.chat_id}`}
    >
      <Text as="p" size="2" weight="medium" className={styles.sessionTitle}>
        Chat {session.chat_id}
      </Text>
      {session.warnings.map((warning) => (
        <Text
          as="p"
          size="1"
          role="status"
          key={warning}
          className={styles.note}
        >
          {warningText[warning]}
        </Text>
      ))}
      {session.tools.map((tool) => (
        <div
          key={tool.tool_call_id}
          role="group"
          aria-labelledby={`${session.chat_id}:${tool.tool_call_id}:label`}
          className={styles.request}
          data-disabled={blocked ? "true" : undefined}
        >
          <Text
            as="p"
            size="1"
            weight="medium"
            id={`${session.chat_id}:${tool.tool_call_id}:label`}
            className={styles.requestTitle}
          >
            {tool.tool_name}: {tool.tool_call_id}
          </Text>
          <pre className={styles.command}>{tool.command}</pre>
          <Text as="p" size="1" className={styles.note}>
            {tool.rule}
          </Text>
          {tool.rule ===
          "Privacy approval required: command read guarded files" ? (
            <Text as="p" size="1" className={styles.note}>
              The command already ran and read guarded files. Accept releases
              its guarded output to the model; reject withholds that output.
              This does not undo the command.
            </Text>
          ) : (
            <Text as="p" size="1" className={styles.note}>
              This responds only to this native confirmation. It does not change
              permission rules or approve other tools.
            </Text>
          )}
          {(["accept", "reject"] as const).map((choice) => (
            <label key={choice} className={styles.choice}>
              <input
                type="radio"
                name={`${session.chat_id}:${tool.tool_call_id}`}
                disabled={blocked}
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
        </div>
      ))}
      {session.browser_request_id && (
        <div
          role="group"
          aria-labelledby={`${session.chat_id}:browser:label`}
          className={styles.request}
          data-disabled={blocked ? "true" : undefined}
        >
          <Text
            as="p"
            size="1"
            weight="medium"
            id={`${session.chat_id}:browser:label`}
            className={styles.requestTitle}
          >
            Browser context for the pending message
          </Text>
          <Text as="p" size="1" className={styles.note}>
            Choose which current browser data to include with your pending
            message. Console, network and screenshots may contain private
            information. Unselected categories are omitted; submitting with none
            selected still sends your message. This is not permission to control
            the browser.
          </Text>
          {browserChoices.map((choice) => (
            <label key={choice} className={styles.choice}>
              <input
                type="checkbox"
                disabled={blocked}
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
          <label className={styles.choice}>
            <input
              type="checkbox"
              disabled={blocked}
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
        </div>
      )}
      {sending && (
        <Text as="p" size="1" role="status" className={styles.note}>
          Submitting to the native owner…
        </Text>
      )}
      {sent && (
        <Text as="p" size="1" role="status" className={styles.note}>
          Response queued, not resolved. Waiting for a fresh native snapshot.
        </Text>
      )}
      {failed && (
        <Text as="p" size="1" role="alert" className={styles.alert}>
          Response not confirmed. Check the native chat before retrying; no
          success is assumed.
        </Text>
      )}
    </Surface>
  );
}

function Approvals({ identity }: { identity: string }) {
  const visible = useIsDocumentVisible();
  const [halted, setHalted] = useState(false);
  const query = useGetBuddyNativeApprovalsQuery(
    { endpoint_identity: identity },
    {
      pollingInterval: visible && !halted ? APPROVAL_POLLING_INTERVAL_MS : 0,
      skipPollingIfUnfocused: true,
      refetchOnMountOrArgChange: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
    },
  );
  const snapshot = query.currentData;
  const failed = Boolean(query.error);
  useEffect(() => {
    setHalted(failed);
  }, [failed]);
  const sessions =
    snapshot?.sessions.filter(
      (session) =>
        session.tools.length ||
        session.browser_request_id ||
        session.warnings.length,
    ) ?? [];
  const warnings = failed ? [] : snapshot?.warnings ?? [];
  const truncated = !failed && Boolean(snapshot?.truncated);
  if (!failed && !truncated && !warnings.length && !sessions.length)
    return null;
  return (
    <Surface
      as="section"
      variant="glass"
      radius="card"
      className={styles.approvals}
      aria-label="Native approvals"
    >
      <BuddySectionHeader
        icon={ShieldCheck}
        label="Native approvals"
        badge={
          sessions.length > 0 ? (
            <Badge tone="accent">{sessions.length}</Badge>
          ) : undefined
        }
        actions={
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void query.refetch()}
          >
            Refresh native approvals
          </Button>
        }
      />
      {failed && (
        <div className={styles.errorRow} role="alert">
          <Icon icon={ShieldAlert} size="sm" tone="danger" />
          <Text as="p" size="1" className={styles.errorText}>
            Native approvals could not be loaded, so approval controls are
            disabled. Pending requests were not resolved — use the native chat
            or retry.
          </Text>
          <Button size="sm" onClick={() => void query.refetch()}>
            Retry
          </Button>
        </div>
      )}
      {truncated && (
        <Text as="p" size="1" role="status" className={styles.note}>
          Partial coverage: more native requests may exist. Open the remaining
          chats.
        </Text>
      )}
      {warnings.map((warning) => (
        <Text
          as="p"
          size="1"
          role="status"
          key={warning}
          className={styles.note}
        >
          {warningText[warning]}
        </Text>
      ))}
      {snapshot &&
        sessions.map((session) => (
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
            disabled={failed || query.isFetching}
            refresh={() => {
              void query.refetch();
            }}
          />
        ))}
    </Surface>
  );
}
export function BuddyNativeApprovals() {
  const identity = useAppSelector(buddyOperatorIdentity);
  return <Approvals key={identity} identity={identity} />;
}
