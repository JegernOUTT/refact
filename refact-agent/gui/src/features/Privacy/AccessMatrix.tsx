import { Badge, StatusDot } from "../../components/ui";
import type {
  PrivacyDestination,
  PrivacyMatchCounts,
  PrivacyZone,
} from "../../services/refact/privacy";
import { zoneAllowsDestination } from "./access";
import styles from "./PrivacySettingsSection.module.css";

export interface AccessMatrixProps {
  zones: PrivacyZone[];
  destinations: PrivacyDestination[];
  matchCounts: PrivacyMatchCounts;
}

export function AccessMatrix({
  destinations,
  matchCounts,
  zones,
}: AccessMatrixProps) {
  return (
    <div className={`${styles.matrixScroll} scrollX`}>
      <table
        aria-label="Zone destination permissions"
        className={styles.matrix}
      >
        <thead>
          <tr>
            <th className={styles.matrixCorner} scope="col">
              Zone
            </th>
            {destinations.map((destination) => (
              <th key={`${destination.kind}:${destination.id}`} scope="col">
                <span className={styles.matrixHeading}>
                  <span>{destination.display_name}</span>
                  <Badge size="xs" tone="muted" variant="outline">
                    {destination.kind.replace(/_/g, " ")}
                  </Badge>
                </span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {zones.map((zone) => (
            <tr key={zone.name}>
              <th className={styles.matrixRowHead} scope="row">
                <span className={styles.matrixRowHeadInner}>
                  <span className={styles.zoneName}>{zone.name}</span>
                  <Badge size="xs" tone="muted" variant="soft">
                    {matchCounts[zone.name] ?? 0}
                  </Badge>
                </span>
              </th>
              {destinations.map((destination) => {
                const allowed = zoneAllowsDestination(zone, destination.id);

                return (
                  <td
                    key={`${zone.name}:${destination.kind}:${destination.id}`}
                  >
                    <span className={styles.matrixCell}>
                      <StatusDot
                        aria-hidden="true"
                        status={allowed ? "success" : "paused"}
                      />
                      <span className={styles.srOnly}>
                        {zone.name} to {destination.display_name}:
                      </span>
                      {allowed ? "Allowed" : "Blocked"}
                    </span>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
