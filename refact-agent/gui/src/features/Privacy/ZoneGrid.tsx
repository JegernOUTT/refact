import { Badge, Button, StatusDot } from "../../components/ui";
import type {
  PrivacyDestination,
  PrivacyZone,
} from "../../services/refact/privacy";
import styles from "./PrivacySettingsSection.module.css";

export interface ZoneGridProps {
  zones: PrivacyZone[];
  destinations: PrivacyDestination[];
  matchCounts: Record<string, number>;
  selectedZoneName: string | null;
  saving: boolean;
  onSelectZone: (zoneName: string) => void;
  onToggle: (zoneName: string, destinationId: string) => void;
}

function isAllowed(zone: PrivacyZone, destinationId: string) {
  return zone.send_to.includes("*") || zone.send_to.includes(destinationId);
}

export function ZoneGrid({
  destinations,
  matchCounts,
  onSelectZone,
  onToggle,
  saving,
  selectedZoneName,
  zones,
}: ZoneGridProps) {
  return (
    <div className={`${styles.gridScroll} scrollX`}>
      <table
        aria-label="Zone destination permissions"
        className={styles.zoneGrid}
      >
        <thead>
          <tr>
            <th scope="col">Zone</th>
            {destinations.map((destination) => (
              <th key={`${destination.kind}:${destination.id}`} scope="col">
                <span className={styles.destinationHeading}>
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
              <th scope="row">
                <Button
                  aria-pressed={selectedZoneName === zone.name}
                  size="sm"
                  variant={selectedZoneName === zone.name ? "soft" : "ghost"}
                  onClick={() => onSelectZone(zone.name)}
                >
                  {zone.name}
                </Button>
                <Badge size="xs" tone="muted" variant="soft">
                  {matchCounts[zone.name]} matches
                </Badge>
              </th>
              {destinations.map((destination) => {
                const allowed = isAllowed(zone, destination.id);
                const action = allowed ? "Deny" : "Allow";

                return (
                  <td
                    key={`${zone.name}:${destination.kind}:${destination.id}`}
                  >
                    <Button
                      aria-label={`${action} ${zone.name} to ${destination.display_name}`}
                      disabled={saving}
                      size="sm"
                      variant={allowed ? "soft" : "ghost"}
                      onClick={() => onToggle(zone.name, destination.id)}
                    >
                      <StatusDot
                        aria-hidden="true"
                        status={allowed ? "success" : "paused"}
                      />
                      {allowed ? "Allowed" : "Blocked"}
                    </Button>
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
