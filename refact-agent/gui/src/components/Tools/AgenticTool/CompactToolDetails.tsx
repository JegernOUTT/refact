import type { ReactNode } from "react";
import { ToolCard } from "../../ui";
import {
  compactFacts,
  type CompactAgenticPresentation,
} from "./compactPresentation";
import styles from "./CompactToolDetails.module.css";

export type CompactToolDetailsProps = {
  facts: CompactAgenticPresentation["facts"];
  navigation?: ReactNode;
  children?: ReactNode;
  detailsLabel?: "More details" | "Technical details";
};

export function CompactToolDetails({
  facts,
  navigation,
  children,
  detailsLabel = "More details",
}: CompactToolDetailsProps) {
  const shown = compactFacts(facts);
  return (
    <div className={styles.root}>
      {shown.length > 0 && (
        <dl className={styles.facts}>
          {shown.map((fact, index) => (
            <div className={styles.fact} key={`${fact.label}-${index}`}>
              <dt className={styles.label}>{fact.label}</dt>
              <dd className={styles.value}>{fact.value}</dd>
            </div>
          ))}
        </dl>
      )}
      {navigation && (
        <nav aria-label="Related navigation" className={styles.navigation}>
          {navigation}
        </nav>
      )}
      {children !== null && children !== undefined && (
        <ToolCard defaultOpen={false} title={detailsLabel}>
          {children}
        </ToolCard>
      )}
    </div>
  );
}
