import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import { Badge, Button, StatusDot, Switch } from "../../components/ui";
import type {
  PrivacyDestination,
  PrivacyDestinationKind,
  PrivacyMatchCounts,
  PrivacyToolAccess,
  PrivacyZone,
} from "../../services/refact/privacy";
import { mcpAllowedForProvider, zoneAllowsDestination } from "./access";
import styles from "./PrivacySettingsSection.module.css";

const GROUP_ORDER: { kind: PrivacyDestinationKind; label: string }[] = [
  { kind: "provider", label: "Model providers" },
  { kind: "mcp", label: "MCP servers" },
  { kind: "completion", label: "Completion" },
  { kind: "subagent_model", label: "Subagent models" },
];

export interface DestinationAccessProps {
  destinations: PrivacyDestination[];
  zones: PrivacyZone[];
  matchCounts: PrivacyMatchCounts;
  toolAccess: PrivacyToolAccess;
  mcpServers: string[];
  saving: boolean;
  onToggleZone: (zoneName: string, destinationId: string) => void;
  onToggleMcp: (providerId: string, server: string) => void;
}

export function DestinationAccess({
  destinations,
  matchCounts,
  mcpServers,
  onToggleMcp,
  onToggleZone,
  saving,
  toolAccess,
  zones,
}: DestinationAccessProps) {
  const groups = useMemo(
    () =>
      GROUP_ORDER.map((group) => ({
        ...group,
        items: destinations.filter(
          (destination) => destination.kind === group.kind,
        ),
      })).filter((group) => group.items.length > 0),
    [destinations],
  );

  const [activeKind, setActiveKind] = useState<PrivacyDestinationKind | null>(
    null,
  );
  const [expanded, setExpanded] = useState<string | null>(null);

  if (groups.length === 0) {
    return (
      <p className={styles.emptyHint}>
        No destinations are configured yet. Add a model provider or an MCP
        server first.
      </p>
    );
  }

  const currentKind =
    activeKind && groups.some((group) => group.kind === activeKind)
      ? activeKind
      : groups[0].kind;
  const currentGroup =
    groups.find((group) => group.kind === currentKind) ?? groups[0];

  return (
    <div className={styles.destinationBlock}>
      <div className={styles.tabRow}>
        {groups.map((group) => (
          <Button
            aria-pressed={group.kind === currentKind}
            key={group.kind}
            size="sm"
            variant={group.kind === currentKind ? "soft" : "ghost"}
            onClick={() => setActiveKind(group.kind)}
          >
            {group.label}
            <Badge size="xs" tone="muted" variant="soft">
              {group.items.length}
            </Badge>
          </Button>
        ))}
      </div>

      <ul className={styles.destinationList}>
        {currentGroup.items.map((destination) => {
          const rowKey = `${destination.kind}:${destination.id}`;
          const isOpen = expanded === rowKey;
          const allowedZones = zones.filter((zone) =>
            zoneAllowsDestination(zone, destination.id),
          );
          const isProvider = destination.kind === "provider";
          const allowedMcp = mcpServers.filter((server) =>
            mcpAllowedForProvider(toolAccess, destination.id, server),
          );

          return (
            <li className={styles.destinationItem} key={rowKey}>
              <button
                aria-expanded={isOpen}
                className={styles.destinationRow}
                type="button"
                onClick={() => setExpanded(isOpen ? null : rowKey)}
              >
                <span className={styles.destinationName}>
                  {isOpen ? (
                    <ChevronDown aria-hidden="true" size={14} />
                  ) : (
                    <ChevronRight aria-hidden="true" size={14} />
                  )}
                  {destination.display_name}
                </span>
                <span className={styles.destinationSummary}>
                  <span className={styles.summaryPart}>
                    <StatusDot
                      aria-hidden="true"
                      status={allowedZones.length > 0 ? "success" : "paused"}
                    />
                    {allowedZones.length}/{zones.length} zones
                  </span>
                  {isProvider && mcpServers.length > 0 ? (
                    <span className={styles.summaryPart}>
                      <StatusDot
                        aria-hidden="true"
                        status={allowedMcp.length > 0 ? "success" : "paused"}
                      />
                      {allowedMcp.length}/{mcpServers.length} MCP
                    </span>
                  ) : null}
                </span>
              </button>

              {isOpen ? (
                <div className={`${styles.destinationPanel} rf-enter`}>
                  <section className={styles.panelColumn}>
                    <h4 className={styles.panelHeading}>
                      May receive files from
                    </h4>
                    {zones.map((zone) => (
                      <Switch
                        aria-label={`Send ${zone.name} to ${destination.display_name}`}
                        checked={zoneAllowsDestination(zone, destination.id)}
                        disabled={saving}
                        key={zone.name}
                        label={
                          <span className={styles.switchLabel}>
                            {zone.name}
                            <span className={styles.switchMeta}>
                              {matchCounts[zone.name] ?? 0} files
                            </span>
                          </span>
                        }
                        onCheckedChange={() =>
                          onToggleZone(zone.name, destination.id)
                        }
                      />
                    ))}
                  </section>

                  {isProvider && mcpServers.length > 0 ? (
                    <section className={styles.panelColumn}>
                      <h4 className={styles.panelHeading}>
                        May use tools from MCP
                      </h4>
                      {mcpServers.map((server) => (
                        <Switch
                          aria-label={`Allow ${destination.display_name} to use ${server}`}
                          checked={mcpAllowedForProvider(
                            toolAccess,
                            destination.id,
                            server,
                          )}
                          disabled={saving}
                          key={server}
                          label={server}
                          onCheckedChange={() =>
                            onToggleMcp(destination.id, server)
                          }
                        />
                      ))}
                    </section>
                  ) : null}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
