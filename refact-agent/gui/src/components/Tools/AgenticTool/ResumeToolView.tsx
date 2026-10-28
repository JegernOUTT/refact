import { humanizeIdentifier } from "../../../utils/displayNames";
import { TranscriptFacts } from "../../ChatContent/TranscriptItem";
import { parseResumeReceipt } from "./resumeReceipt";
import styles from "./AgenticTool.module.css";

function eligibility(
  receipt: NonNullable<ReturnType<typeof parseResumeReceipt>>,
) {
  if (receipt.can_raise_and_resume) return "Can raise the step cap";
  if (receipt.can_continue) return "Can continue";
  if (receipt.can_resume) return "Can resume";
  return "Not resumable";
}

export function ResumeToolView({
  lifecycle = false,
  content,
}: {
  lifecycle?: boolean;
  content: string | null;
}) {
  const receipt = parseResumeReceipt(content, lifecycle);
  if (receipt === null) return null;
  const cap = receipt.cap_stop;
  const facts = [
    { label: "Action", value: humanizeIdentifier(receipt.action) },
    { label: "State", value: humanizeIdentifier(receipt.status) },
    {
      label: "Steps",
      value:
        receipt.max_steps !== null
          ? `${receipt.step_count} / ${receipt.max_steps}`
          : String(receipt.step_count),
    },
    { label: "Run", value: String(receipt.run_generation) },
    ...(receipt.action === "inspect"
      ? [{ label: "Eligibility", value: eligibility(receipt) }]
      : []),
  ];
  return (
    <section
      aria-label={lifecycle ? "Child lifecycle" : "Child resume"}
      className={styles.section}
    >
      {receipt.summary && <p>{receipt.summary}</p>}
      <TranscriptFacts facts={facts} />
      {cap && (
        <p aria-label="Execution cap">
          {humanizeIdentifier(cap.key)} cap reached · {cap.used} / {cap.limit}
        </p>
      )}
      {!receipt.resume_configuration_available && (
        <p role="status">Saved runtime configuration unavailable.</p>
      )}
      {!receipt.original_scope_available && (
        <p role="status">Original execution scope unavailable.</p>
      )}
    </section>
  );
}
