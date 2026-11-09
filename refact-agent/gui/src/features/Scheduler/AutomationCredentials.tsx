import React, { useCallback, useEffect, useState } from "react";
import { Flex, Heading, Text } from "@radix-ui/themes";
import { Button, Field, FieldText } from "../../components/ui";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  credentialHostAvailable,
  listCredentials,
  mutateCredential,
  type CredentialCatalog,
  type CredentialIdentity,
  type CredentialMetadata,
  type CredentialMutation,
} from "../../services/refact/automationCredentialsApi";
import styles from "./AutomationCredentials.module.css";

const emptyIdentity: CredentialIdentity = {
  id: "",
  origin: "",
  header: "authorization",
  tenant: "",
  principal: "",
  scope: [],
};
type Selection = {
  operation: "rotate" | "remove";
  metadata: CredentialMetadata;
};

export function AutomationCredentials() {
  const token = useAppSelector((state) => state.config.apiKey);
  const available = credentialHostAvailable();
  const [catalog, setCatalog] = useState<CredentialCatalog | null>(null);
  const [identity, setIdentity] = useState(emptyIdentity);
  // Secret has exactly one ephemeral React state owner. Never dispatch it,
  // persist it, log it, include it in errors, or send it through model tools.
  const [secret, setSecret] = useState("");
  const [selection, setSelection] = useState<Selection | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const refresh = useCallback(async () => {
    setSecret("");
    setSelection(null);
    setConfirmed(false);
    try {
      setCatalog(await listCredentials(token));
      setNotice("");
    } catch {
      setCatalog(null);
      setNotice(
        "Credential metadata unavailable. Use the trusted local worker with exactly one project and host authorization.",
      );
    }
  }, [token]);
  useEffect(() => {
    if (available) void refresh();
  }, [available, refresh]);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!catalog?.supported || !confirmed || busy) return;
    const base = { project_root: catalog.project_root };
    const request: CredentialMutation = selection
      ? selection.operation === "remove"
        ? {
            ...base,
            operation: "remove",
            id: selection.metadata.id,
            expected_secret_revision: selection.metadata.secret_revision,
          }
        : {
            ...base,
            operation: "rotate",
            id: selection.metadata.id,
            expected_secret_revision: selection.metadata.secret_revision,
            secret,
          }
      : { ...base, operation: "create", identity, secret };
    // Clear before starting/awaiting the network operation, including failures.
    setSecret("");
    setConfirmed(false);
    setBusy(true);
    setNotice("");
    try {
      await mutateCredential(request, token);
      await refresh();
      setNotice(
        "Credential operation completed. Enrollment does not authorize any source.",
      );
    } catch {
      setNotice(
        "Credential operation refused. Refresh metadata before retrying; verify access, revision, identity, value limits, and private storage.",
      );
    } finally {
      setBusy(false);
    }
  }
  function choose(
    operation: "rotate" | "remove",
    metadata: CredentialMetadata,
  ) {
    setSecret("");
    setConfirmed(false);
    setSelection({ operation, metadata });
    setNotice("");
  }
  function cancel() {
    setSecret("");
    setSelection(null);
    setConfirmed(false);
  }
  return (
    <section
      className={styles.section}
      aria-label="Private automation credentials"
    >
      <Heading size="3">Private credentials</Heading>
      <Text size="2">
        Host operator only. Values are stored outside the project as
        OS-permission-protected plaintext on supported Unix hosts, not
        encrypted. Never paste secrets into chat or manifests. Enrollment grants
        no source access; exact source binding requires separate human approval.
      </Text>
      {!available ? (
        <Text role="status">
          Open the trusted local worker’s own top-level browser UI to manage
          credentials. IDE webviews, App frames, and remote origins are not
          supported here.
        </Text>
      ) : (
        <>
          <Button disabled={busy} onClick={() => void refresh()}>
            Refresh credential metadata
          </Button>
          {catalog && (
            <Text size="2" className={styles.metadata}>
              Project: {catalog.project_root}
            </Text>
          )}
          {catalog && !catalog.supported && (
            <Text role="status">
              Private credential enrollment is unsupported on this platform.
            </Text>
          )}
          {catalog?.supported && (
            <>
              {catalog.credentials.length === 0 && (
                <Text size="2">No private credentials enrolled.</Text>
              )}
              {catalog.credentials.map((item) => (
                <div key={item.id} className={styles.entry}>
                  <Text weight="bold">{item.id}</Text>
                  <Text size="2" className={styles.metadata}>
                    {item.origin} · {item.header} · Principal: {item.principal}{" "}
                    · Tenant: {item.tenant || "None"} · Scope:{" "}
                    {item.scope.join(", ") || "None"}
                  </Text>
                  <Text size="1" className={styles.metadata}>
                    Identity revision: {item.identity_revision} · Secret
                    revision: {item.secret_revision}
                  </Text>
                  <Flex gap="2" wrap="wrap">
                    <Button
                      disabled={busy}
                      onClick={() => choose("rotate", item)}
                    >
                      Rotate {item.id}
                    </Button>
                    <Button
                      disabled={busy}
                      onClick={() => choose("remove", item)}
                    >
                      Revoke {item.id}
                    </Button>
                  </Flex>
                </div>
              ))}
              <form
                onSubmit={(event) => void submit(event)}
                className={styles.form}
                autoComplete="off"
              >
                <Heading size="2">
                  {selection
                    ? `${
                        selection.operation === "rotate" ? "Rotate" : "Revoke"
                      } ${selection.metadata.id}`
                    : "Enroll a credential"}
                </Heading>
                {!selection &&
                  (
                    Object.keys(emptyIdentity) as (keyof CredentialIdentity)[]
                  ).map((key) => (
                    <Field
                      key={key}
                      label={
                        key === "id"
                          ? "Reference ID"
                          : key === "scope"
                            ? "Scope (comma-separated)"
                            : key === "origin"
                              ? "Exact origin (scheme, host, port)"
                              : key === "header"
                                ? "Header name"
                                : key === "principal"
                                  ? "Principal"
                                  : "Tenant"
                      }
                      htmlFor={`credential-${key}`}
                    >
                      <FieldText
                        id={`credential-${key}`}
                        disabled={busy}
                        required={key !== "scope" && key !== "tenant"}
                        value={
                          key === "scope"
                            ? identity.scope.join(",")
                            : identity[key]
                        }
                        onChange={(value) =>
                          setIdentity((current) => ({
                            ...current,
                            [key]:
                              key === "scope"
                                ? value
                                    .split(",")
                                    .map((s) => s.trim())
                                    .filter(Boolean)
                                : value,
                          }))
                        }
                      />
                    </Field>
                  ))}
                {selection && (
                  <Text size="2" className={styles.metadata}>
                    Exact secret revision: {selection.metadata.secret_revision}.{" "}
                    {selection.operation === "remove"
                      ? "Revocation removes this value and all source binding grants."
                      : "Rotation preserves the existing identity and scope; stale revisions are refused."}
                  </Text>
                )}
                {selection?.operation !== "remove" && (
                  <Field
                    label="Secret value"
                    htmlFor="credential-secret"
                    helper="Maximum 4096 bytes. Sent directly to this worker, never to Redux or a model. Your browser network inspector can show your own request."
                  >
                    <FieldText
                      id="credential-secret"
                      type="password"
                      autoComplete="new-password"
                      spellCheck={false}
                      maxLength={4096}
                      value={secret}
                      onChange={setSecret}
                      disabled={busy}
                      required
                    />
                  </Field>
                )}
                <label className={styles.confirmation}>
                  <input
                    type="checkbox"
                    checked={confirmed}
                    disabled={busy}
                    onChange={(event) =>
                      setConfirmed(event.currentTarget.checked)
                    }
                  />
                  I confirm this exact host credential operation for this
                  project. This is not source authorization.
                </label>
                <Flex gap="2" wrap="wrap">
                  <Button type="submit" disabled={busy || !confirmed}>
                    {busy
                      ? "Saving…"
                      : selection?.operation === "remove"
                        ? "Confirm revocation"
                        : selection
                          ? "Confirm rotation"
                          : "Confirm enrollment"}
                  </Button>
                  <Button type="button" disabled={busy} onClick={cancel}>
                    Cancel / clear secret
                  </Button>
                </Flex>
              </form>
            </>
          )}
        </>
      )}
      {notice && <Text role="status">{notice}</Text>}
    </section>
  );
}
