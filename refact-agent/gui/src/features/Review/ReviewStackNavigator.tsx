import type { CSSProperties } from "react";
import { Icon, Tooltip } from "../../components/ui";
import { GitBranch, Layers, Route } from "lucide-react";
import { Select } from "../../components/ui/Select/Select";
import { SegmentedControl } from "../../components/ui/SegmentedControl/SegmentedControl";
import type { StackManifest, StackMember } from "../../services/refact/review";
import styles from "./ReviewStackNavigator.module.css";

export type ReviewStackNavigatorProps = {
  stack: StackManifest;
  selectedMemberId: string | null;
  onSelect: (memberId: string) => void;
  comparison: "parent" | "cumulative";
  onComparisonChange: (comparison: "parent" | "cumulative") => void;
  activeReviewId?: string;
};

function unavailableReason(stack: StackManifest, member: StackMember) {
  if (!stack.scope_choice.confirmed)
    return "Scope choice pending in owning chat";
  if (
    stack.scope_choice.scope === "single" &&
    stack.scope_choice.selected_member_id !== member.id
  )
    return "outside selected scope";
  if (member.state !== "ready")
    return `${
      member.state === "removed" ? "Historical · removed" : member.state
    }${member.reason ? ` · ${member.reason}` : ""}`;
  return null;
}

function prNumber(member: StackMember) {
  return member.pr_identity ? `#${member.pr_identity.number} ` : "";
}

/** Parent links, not array indices, determine fork indentation. Cycle-safe for untrusted input. */
function depthOf(member: StackMember, members: Map<string, StackMember>) {
  const seen = new Set([member.id]);
  let depth = 0;
  let parent = member.parent_member_id;
  while (parent && !seen.has(parent) && members.has(parent)) {
    seen.add(parent);
    depth += 1;
    parent = members.get(parent)?.parent_member_id ?? null;
  }
  return depth;
}

/** Controlled browsing only. No context mutation, acquisition, execution, or scope widening. */
export function ReviewStackNavigator({
  stack,
  selectedMemberId,
  onSelect,
  comparison,
  onComparisonChange,
  activeReviewId,
}: ReviewStackNavigatorProps) {
  const members = new Map(stack.members.map((member) => [member.id, member]));
  const selected = selectedMemberId ? members.get(selectedMemberId) : undefined;
  const recorded = stack.members.some(
    (member) => member.reason?.startsWith("entry:recorded_comparisons:"),
  );
  const cumulativeReason =
    stack.scope_choice.scope === "whole"
      ? "Whole stack"
      : "Whole stack comparison is outside selected scope";
  const position =
    stack.members.findIndex((member) => member.id === selectedMemberId) + 1;
  const execution = activeReviewId
    ? stack.members.find((member) => member.review_id === activeReviewId)
    : undefined;
  const selectMember = (id: string) => {
    const member = members.get(id);
    if (member && !unavailableReason(stack, member)) onSelect(id);
  };
  const metadata = `Root: ${stack.root_base_ref} (${
    stack.root_base_oid ?? "unresolved"
  })${
    selected
      ? `\nBase: ${selected.declared_base_ref} (${
          selected.resolved_base_oid ?? "unresolved"
        })\nHead: ${selected.head_oid ?? "unresolved"}`
      : ""
  }\nSource: ${stack.source.kind}${
    stack.source.reference ? ` · ${stack.source.reference}` : ""
  }`;
  return (
    <nav className={styles.root} aria-label="Review stack">
      <Icon icon={GitBranch} size="sm" />
      <Select value={selectedMemberId ?? ""} onValueChange={selectMember}>
        <Select.Trigger
          className={styles.trigger}
          aria-label="Browse stack branch"
          placeholder="Browse branch"
          title={selected?.branch_ref}
        >
          {selected && `${prNumber(selected)}${selected.branch_ref}`}
        </Select.Trigger>
        <Select.Content
          maxHeight="min(60vh, 400px)"
          maxWidth="min(90vw, 520px)"
        >
          {stack.members.map((member) => {
            const parent = recorded
              ? member.declared_base_ref
              : member.parent_member_id
                ? members.get(member.parent_member_id)?.branch_ref ??
                  member.parent_member_id
                : stack.root_base_ref;
            const reason = unavailableReason(stack, member);
            const isExecution = Boolean(
              activeReviewId && member.review_id === activeReviewId,
            );
            return (
              <Select.Item
                key={member.id}
                value={member.id}
                disabled={Boolean(reason)}
                textValue={member.branch_ref}
                aria-label={`${member.branch_ref}, ${
                  recorded ? "saved base" : "parent"
                } ${parent}${isExecution ? ", current execution member" : ""}${
                  reason ? `, ${reason}` : ""
                }`}
                title={`${member.branch_ref} ← ${parent}${
                  reason ? ` · ${reason}` : ""
                }`}
                className={styles.item}
                style={
                  {
                    "--stack-depth": Math.min(depthOf(member, members), 10),
                  } as CSSProperties
                }
              >
                <span className={styles.member}>
                  <span className={styles.branch}>
                    {member.parent_member_id ? "↳ " : ""}
                    {prNumber(member)}
                    {member.branch_ref}
                  </span>
                  {isExecution && (
                    <span
                      className={styles.execution}
                      aria-hidden="true"
                      title="Current execution member"
                    >
                      ●
                    </span>
                  )}
                  <span className={styles.parent}>← {parent}</span>
                  {reason && <span className={styles.state}>{reason}</span>}
                </span>
              </Select.Item>
            );
          })}
        </Select.Content>
      </Select>
      <span className={styles.metadata} title={metadata}>
        {position > 0
          ? `${position} of ${stack.members.length}`
          : `${stack.members.length} in the stack`}
      </span>
      {execution && execution.id !== selectedMemberId && (
        <span
          className={styles.execution}
          title={`Current execution member: ${execution.branch_ref}`}
          aria-label={`Current execution member: ${execution.branch_ref}`}
        >
          ●
        </span>
      )}
      {!recorded && (
        <SegmentedControl
          size="sm"
          layout="content"
          aria-label="Stack comparison"
          value={comparison}
          onValueChange={(value) => {
            if (value === "parent" || value === "cumulative")
              onComparisonChange(value);
          }}
          options={[
            {
              value: "parent",
              label: (
                <Tooltip content="Parent-relative">
                  <span>
                    <Icon icon={Route} size="sm" />
                  </span>
                </Tooltip>
              ),
              iconOnly: true,
              ariaLabel: "Parent-relative",
            },
            {
              value: "cumulative",
              disabled:
                !stack.scope_choice.confirmed ||
                stack.scope_choice.scope !== "whole",
              label: (
                <Tooltip content={cumulativeReason}>
                  <span title={cumulativeReason}>
                    <Icon icon={Layers} size="sm" />
                  </span>
                </Tooltip>
              ),
              iconOnly: true,
              ariaLabel: "Whole stack",
            },
          ]}
        />
      )}
    </nav>
  );
}
