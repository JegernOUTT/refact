import React, { useMemo } from "react";
import { Boxes, X } from "lucide-react";
import { Dialog } from "../ui";
import { KitIconButton } from "../ComposerKit";
import type { StagedAppContext } from "../../features/Apps/appsSlice";
import {
  APP_CONTEXT_MAX_PER_MESSAGE,
  planAppContexts,
  type DeferredReason,
  type FrozenAppContext,
} from "./appContext";
import styles from "./AppContextTray.module.css";

type AppContextTrayProps = {
  staged: readonly StagedAppContext[];
  onRemove: (id: string) => void;
};

const DEFERRED_LABEL: Record<DeferredReason, string> = {
  limit: "next message",
  invalid_ticket: "not sendable",
};

const DEFERRED_NOTE: Record<DeferredReason, string> = {
  limit: `Only ${APP_CONTEXT_MAX_PER_MESSAGE} app contexts are sent per message. This one stays staged for the next message.`,
  invalid_ticket:
    "This context has no valid host ticket and will not be sent. Remove it and stage it again from the app.",
};

const AppContextChip: React.FC<{
  context: FrozenAppContext;
  deferred: DeferredReason | null;
  onRemove: () => void;
}> = ({ context, deferred, onRemove }) => {
  const meta = [
    `App ${context.app_id}`,
    `Binding ${context.binding_id}`,
    ...(context.view_id ? [`View ${context.view_id}`] : []),
    `Revision ${context.revision}`,
    ...(context.captured_at ? [`Captured ${context.captured_at}`] : []),
  ];
  return (
    <div
      className={`${styles.chip} rf-enter`}
      data-deferred={deferred !== null}
    >
      <Dialog>
        <Dialog.Trigger asChild>
          <button
            type="button"
            className={styles.trigger}
            aria-label={`Preview app context: ${context.label}`}
          >
            <Boxes aria-hidden="true" className={styles.icon} />
            <span className={styles.label}>{context.label}</span>
            <span className={styles.meta}>
              {deferred ? DEFERRED_LABEL[deferred] : context.app_id}
            </span>
          </button>
        </Dialog.Trigger>
        <Dialog.Content maxWidth="640px">
          <Dialog.Title>{context.label}</Dialog.Title>
          <Dialog.Description>
            Untrusted app context. Preview only: on send the engine attaches
            this content as data from the staged ticket, not as instructions or
            privileges.
          </Dialog.Description>
          <ul className={styles.attribution}>
            {meta.map((line) => (
              <li key={line}>{line}</li>
            ))}
            {context.source_refs.map((ref) => (
              <li key={ref}>Source claimed by app (unverified): {ref}</li>
            ))}
          </ul>
          <pre className={styles.preview}>{context.text}</pre>
          {deferred ? (
            <p className={styles.note}>{DEFERRED_NOTE[deferred]}</p>
          ) : null}
        </Dialog.Content>
      </Dialog>
      <span className={styles.actions}>
        <KitIconButton
          aria-label={`Remove app context: ${context.label}`}
          icon={X}
          onClick={onRemove}
        />
      </span>
    </div>
  );
};

export const AppContextTray: React.FC<AppContextTrayProps> = ({
  staged,
  onRemove,
}) => {
  const items = useMemo(() => {
    const plan = planAppContexts(staged);
    return [
      ...plan.included.map((context) => ({
        context,
        deferred: null as DeferredReason | null,
      })),
      ...plan.deferred.map(({ context, reason }) => ({
        context,
        deferred: reason as DeferredReason | null,
      })),
    ];
  }, [staged]);

  if (items.length === 0) return null;

  return (
    <div className={styles.tray} data-testid="app-context-tray">
      {items.map(({ context, deferred }) => (
        <AppContextChip
          key={context.id}
          context={context}
          deferred={deferred}
          onRemove={() => onRemove(context.id)}
        />
      ))}
    </div>
  );
};
