import { humanizeIdentifier } from "../../utils/displayNames";
import { useState } from "react";
import { Badge, Button, Dialog, Select, FieldText } from "../../components/ui";
import {
  useAttachEngineMutation,
  useDetachEngineMutation,
  useGetEnginesQuery,
} from "../../services/refact/daemon";
import { useObservationPolling } from "./useObservationPolling";
import styles from "./Console.module.css";

export function EnginesPage() {
  const query = useGetEnginesQuery(undefined, useObservationPolling());
  const [attach, attaching] = useAttachEngineMutation();
  const [detach, detaching] = useDetachEngineMutation();
  const [open, setOpen] = useState(false);
  const [host, setHost] = useState<"127.0.0.1" | "::1">("127.0.0.1");
  const [port, setPort] = useState("8001");
  const [label, setLabel] = useState("");
  const [error, setError] = useState<string | null>(null);
  const valid =
    /^\d+$/.test(port) && Number(port) >= 1 && Number(port) <= 65535;
  return (
    <section className={styles.page}>
      <header className={styles.header}>
        <div>
          <h2>Engines</h2>
          <p>Managed project workers and explicitly connected local engines.</p>
        </div>
        <Button
          onClick={() => {
            setError(null);
            setOpen(true);
          }}
        >
          Connect engine
        </Button>
      </header>
      <p>
        Attached engines are observe-only. Disconnecting removes only the
        attachment, not the engine. Daemon restart clears attachments.
      </p>
      {query.isLoading && <p>Loading engines…</p>}
      {query.isError && (
        <p role="alert">
          Engine observation unavailable.{" "}
          {query.data
            ? "Showing stale observations."
            : "No observation available."}
        </p>
      )}
      {(["managed", "attached"] as const).map((kind) => (
        <section key={kind}>
          <h3>{kind === "managed" ? "Managed" : "Attached"}</h3>
          {query.data?.engines
            .filter((engine) => engine.kind === kind)
            .map((engine) => (
              <article className={styles.row} key={engine.id}>
                <div className={styles.identity}>
                  <strong>{engine.label}</strong>
                  <span className={styles.path}>
                    {engine.root ?? engine.id}
                  </span>
                  <span className={styles.muted}>
                    {engine.http_port
                      ? `Port ${engine.http_port}`
                      : "No active port"}
                  </span>
                </div>
                <div className={styles.identity}>
                  <Badge tone={engine.error ? "warning" : "muted"}>
                    {humanizeIdentifier(engine.state)}
                  </Badge>
                  <span className={styles.muted}>
                    {kind === "attached" ? "Observe-only" : "Daemon managed"}
                  </span>
                  <p>
                    Observed:{" "}
                    {engine.observed_at_ms
                      ? new Date(engine.observed_at_ms).toLocaleTimeString()
                      : "Unknown"}
                    {query.isError ? " · stale" : ""}
                  </p>
                  {engine.error && <p>{engine.error}</p>}
                </div>
                {kind === "attached" ? (
                  <Button
                    variant="ghost"
                    disabled={detaching.isLoading}
                    onClick={() => {
                      void detach(engine.id)
                        .unwrap()
                        .catch(() =>
                          setError(
                            "Disconnect failed. Attachment remains visible until confirmed.",
                          ),
                        );
                    }}
                  >
                    Disconnect
                  </Button>
                ) : (
                  <span className={styles.muted}>Manage in Projects</span>
                )}
              </article>
            ))}
          {query.data &&
            !query.data.engines.some((engine) => engine.kind === kind) && (
              <p>No {kind} engines.</p>
            )}
        </section>
      ))}
      {!open && error && <p role="alert">{error}</p>}
      <Dialog open={open} onOpenChange={setOpen}>
        <Dialog.Content>
          <Dialog.Title>Connect local engine</Dialog.Title>
          <Dialog.Description>
            Observe an existing engine on numeric loopback.
            Authentication-protected engines cannot be connected in this
            version. No engine is started.
          </Dialog.Description>
          <form
            id="connect-engine"
            className={styles.form}
            onSubmit={(event) => {
              event.preventDefault();
              if (!valid) return;
              setError(null);
              void attach({
                host,
                port: Number(port),
                ...(label.trim() ? { label: label.trim() } : {}),
              })
                .unwrap()
                .then(() => setOpen(false))
                .catch(() =>
                  setError(
                    "Connection failed. Check the port, engine availability and authentication requirements.",
                  ),
                );
            }}
          >
            <label>
              Host
              <Select
                value={host}
                onValueChange={(value) =>
                  setHost(value === "::1" ? "::1" : "127.0.0.1")
                }
              >
                <Select.Trigger aria-label="Host" />
                <Select.Content>
                  <Select.Item value="127.0.0.1">127.0.0.1</Select.Item>
                  <Select.Item value="::1">::1</Select.Item>
                </Select.Content>
              </Select>
            </label>
            <label>
              Port
              <FieldText
                type="number"
                min={1}
                max={65535}
                value={port}
                onChange={setPort}
                required
              />
            </label>
            <label>
              Label (optional)
              <FieldText value={label} maxLength={120} onChange={setLabel} />
            </label>
            {error && <p role="alert">{error}</p>}
          </form>
          <Dialog.Footer>
            <Button variant="ghost" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              form="connect-engine"
              disabled={!valid}
              loading={attaching.isLoading}
            >
              Connect
            </Button>
          </Dialog.Footer>
        </Dialog.Content>
      </Dialog>
    </section>
  );
}
