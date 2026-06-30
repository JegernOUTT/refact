import React, { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Badge, Button } from "../../components/ui";
import { useCopyToClipboard } from "../../hooks";
import type { CronTask } from "../../services/refact/schedulerApi";
import styles from "./Scheduler.module.css";

type WebhookManagerProps = {
  task: CronTask;
};

function inboundHookPath(hookId: string | null): string {
  return hookId ? `/hooks/${encodeURIComponent(hookId)}` : "/hooks/:name";
}

export const WebhookManager: React.FC<WebhookManagerProps> = ({ task }) => {
  const copyToClipboard = useCopyToClipboard();
  const [copied, setCopied] = useState(false);
  const rawHookId = task.hook_id?.trim();
  const hookId = rawHookId && rawHookId.length > 0 ? rawHookId : null;
  const path = inboundHookPath(hookId);

  if (task.trigger_kind !== "webhook") return null;

  const handleCopy = () => {
    copyToClipboard(path);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1600);
  };

  return (
    <section className={styles.webhookManager} aria-label="Webhook management">
      <div className={styles.webhookHeader}>
        <span className={styles.webhookTitle}>Webhook trigger</span>
        <Badge tone={hookId ? "accent" : "warning"}>
          {hookId ? `hook_id: ${hookId}` : "hook_id unavailable"}
        </Badge>
      </div>
      <p className={styles.webhookHint}>
        The daemon owns the webhook origin. Use this path with the daemon host,
        or configure the named hook in daemon settings.
      </p>
      <div className={styles.webhookPathRow}>
        <code className={styles.webhookPath}>{path}</code>
        <Button
          type="button"
          variant="soft"
          size="sm"
          leftIcon={copied ? Check : Copy}
          onClick={handleCopy}
        >
          {copied ? "Copied" : "Copy path"}
        </Button>
      </div>
    </section>
  );
};
