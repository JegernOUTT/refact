import classNames from "classnames";
import { Plus, Trash2 } from "lucide-react";
import { Badge, Button, FieldText, IconButton } from "../../components/ui";
import styles from "./ShellRuleList.module.css";

export type ShellRuleTone = "deny" | "ask" | "allow";

export interface ShellRuleListProps {
  title: string;
  hint: string;
  tone: ShellRuleTone;
  rules: string[];
  savedRules: string[];
  disabled: boolean;
  onChange: (rules: string[]) => void;
}

const PLACEHOLDER: Record<ShellRuleTone, string> = {
  deny: "rm -rf *",
  ask: "git push*",
  allow: "ls",
};

export function ShellRuleList({
  disabled,
  hint,
  onChange,
  rules,
  savedRules,
  title,
  tone,
}: ShellRuleListProps) {
  const dirty =
    rules.length !== savedRules.length ||
    rules.some((rule, index) => rule !== savedRules[index]);

  const updateRule = (index: number, value: string) => {
    onChange(rules.map((rule, i) => (i === index ? value : rule)));
  };

  const removeRule = (index: number) => {
    onChange(rules.filter((_, i) => i !== index));
  };

  return (
    <section className={classNames(styles.block, styles[tone])}>
      <div className={styles.header}>
        <h3 className={styles.title}>
          <span aria-hidden="true" className={styles.marker} />
          {title}
        </h3>
        <Badge tone="muted">{rules.length}</Badge>
      </div>
      <p className={styles.hint}>{hint}</p>

      {rules.length === 0 ? (
        <p className={styles.empty}>No rules yet.</p>
      ) : (
        <div className={styles.rows}>
          {rules.map((rule, index) => (
            <div className={styles.row} key={index}>
              <FieldText
                aria-label={`${title} rule ${index + 1}`}
                className={styles.input}
                disabled={disabled}
                placeholder={PLACEHOLDER[tone]}
                spellCheck={false}
                value={rule}
                onChange={(value) => updateRule(index, value)}
              />
              <IconButton
                aria-label={`Remove rule ${index + 1} from ${title}`}
                disabled={disabled}
                icon={Trash2}
                size="sm"
                variant="danger"
                onClick={() => removeRule(index)}
              />
            </div>
          ))}
        </div>
      )}

      <div className={styles.actions}>
        <Button
          disabled={disabled}
          leftIcon={Plus}
          size="sm"
          variant="soft"
          onClick={() => onChange([...rules, ""])}
        >
          Add rule
        </Button>
        {dirty ? (
          <Button
            disabled={disabled}
            size="sm"
            variant="ghost"
            onClick={() => onChange([...savedRules])}
          >
            Revert
          </Button>
        ) : null}
      </div>
    </section>
  );
}
