import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Plus } from "lucide-react";

import {
  Badge,
  Button,
  ErrorState,
  LoadingState,
  SaveStatus,
  SettingItem,
  StatusDot,
  Switch,
} from "../../components/ui";
import { useReducedMotion } from "../../hooks";
import {
  type PrivacyPolicy,
  type PrivacyZone,
  useGetPrivacyPolicyQuery,
  useGetPrivacyStatusQuery,
  useUpdatePrivacyPolicyMutation,
} from "../../services/refact/privacy";
import { SettingsGroup, SettingsSection } from "../Settings/SettingsSection";
import { isCatchAllZone, mcpAllowedForProvider } from "./access";
import { AccessMatrix } from "./AccessMatrix";
import { DestinationAccess } from "./DestinationAccess";
import { PatternChips } from "./PatternChips";
import { ZoneCard } from "./ZoneCard";
import styles from "./PrivacySettingsSection.module.css";

function errorText(error: unknown) {
  if (error && typeof error === "object" && "data" in error) {
    const data = (error as { data: unknown }).data;
    if (typeof data === "string") return data;
    if (data && typeof data === "object" && "detail" in data) {
      return String((data as { detail: unknown }).detail);
    }
  }
  return "The privacy policy could not be saved.";
}

function toggleDestination(
  policy: PrivacyPolicy,
  zoneName: string,
  destinationId: string,
  allDestinationIds: string[],
): PrivacyPolicy {
  return {
    ...policy,
    zones: policy.zones.map((zone) => {
      if (zone.name !== zoneName) return zone;

      const allowed =
        zone.send_to.includes("*") || zone.send_to.includes(destinationId);
      const sendTo = zone.send_to.includes("*")
        ? allDestinationIds.filter((id) => id !== destinationId)
        : allowed
          ? zone.send_to.filter((id) => id !== destinationId)
          : [...zone.send_to, destinationId];

      return { ...zone, send_to: sendTo };
    }),
  };
}

function toggleMcpAccess(
  policy: PrivacyPolicy,
  providerId: string,
  server: string,
  allServers: string[],
): PrivacyPolicy {
  const current = policy.tool_access.providers[providerId]?.mcp ?? ["*"];
  const allowed = current.includes("*") || current.includes(server);
  const next = current.includes("*")
    ? allServers.filter((id) => id !== server)
    : allowed
      ? current.filter((id) => id !== server)
      : [...current, server];
  const coversEverything = allServers.every((id) => next.includes(id));
  const mcp = coversEverything ? ["*"] : next;

  return {
    ...policy,
    tool_access: {
      ...policy.tool_access,
      providers: { ...policy.tool_access.providers, [providerId]: { mcp } },
    },
  };
}

function nextZoneName(zones: PrivacyZone[]) {
  const taken = new Set(zones.map((zone) => zone.name));
  let index = 1;
  let candidate = "new_zone";
  while (taken.has(candidate)) {
    index += 1;
    candidate = `new_zone_${String(index)}`;
  }
  return candidate;
}

export function PrivacySettingsSection() {
  const policyQuery = useGetPrivacyPolicyQuery(undefined);
  const statusQuery = useGetPrivacyStatusQuery(undefined);
  const [updatePolicy, updateState] = useUpdatePrivacyPolicyMutation();
  const [saveError, setSaveError] = useState<string | null>(null);
  const [matrixOpen, setMatrixOpen] = useState(false);
  const matrixRef = useRef<HTMLDivElement>(null);
  const matrixWasOpenRef = useRef(false);
  const reducedMotion = useReducedMotion();

  useEffect(() => {
    if (matrixOpen && !matrixWasOpenRef.current) {
      matrixRef.current?.scrollIntoView({
        block: "nearest",
        behavior: reducedMotion ? "auto" : "smooth",
      });
    }
    matrixWasOpenRef.current = matrixOpen;
  }, [matrixOpen, reducedMotion]);

  const data = policyQuery.data;

  const mcpServers = useMemo(
    () =>
      (data?.destinations ?? [])
        .filter((destination) => destination.kind === "mcp")
        .map((destination) => destination.id),
    [data],
  );

  const save = useCallback(
    async (policy: PrivacyPolicy) => {
      setSaveError(null);
      try {
        await updatePolicy(policy).unwrap();
      } catch (error) {
        setSaveError(errorText(error));
      }
    },
    [updatePolicy],
  );

  const handleZoneToggle = useCallback(
    (zoneName: string, destinationId: string) => {
      if (!data) return;
      void save(
        toggleDestination(
          data.policy,
          zoneName,
          destinationId,
          data.destinations.map((destination) => destination.id),
        ),
      );
    },
    [data, save],
  );

  const handleMcpToggle = useCallback(
    (providerId: string, server: string) => {
      if (!data) return;
      void save(toggleMcpAccess(data.policy, providerId, server, mcpServers));
    },
    [data, mcpServers, save],
  );

  const updateZone = useCallback(
    (zoneName: string, patch: Partial<PrivacyZone>) => {
      if (!data) return;
      void save({
        ...data.policy,
        zones: data.policy.zones.map((zone) =>
          zone.name === zoneName ? { ...zone, ...patch } : zone,
        ),
      });
    },
    [data, save],
  );

  const removeZone = useCallback(
    (zoneName: string) => {
      if (!data) return;
      void save({
        ...data.policy,
        zones: data.policy.zones.filter((zone) => zone.name !== zoneName),
      });
    },
    [data, save],
  );

  const addZone = useCallback(() => {
    if (!data) return;
    const zone: PrivacyZone = {
      name: nextZoneName(data.policy.zones),
      patterns: [],
      send_to: [],
      on_shell_read: "withhold",
    };
    void save({
      ...data.policy,
      zones: [zone, ...data.policy.zones],
    });
  }, [data, save]);

  if (policyQuery.isLoading) {
    return (
      <SettingsSection
        title="Privacy & Access"
        description="Decide which files leave your machine, and what each destination is allowed to touch."
      >
        <LoadingState label="Loading privacy policy" variant="full" />
      </SettingsSection>
    );
  }

  if (policyQuery.isError || !data) {
    return (
      <SettingsSection
        title="Privacy & Access"
        description="Decide which files leave your machine, and what each destination is allowed to touch."
      >
        <ErrorState
          title="Failed to load privacy policy"
          description="The privacy policy endpoint could not be reached."
          retry={
            <Button variant="soft" onClick={() => void policyQuery.refetch()}>
              Retry
            </Button>
          }
          variant="full"
        />
      </SettingsSection>
    );
  }

  const configError = data.error ?? statusQuery.data?.config_error;
  const observation = statusQuery.data?.observation;
  const observationDescription = statusQuery.isError
    ? "Observation status could not be checked."
    : observation?.platform_supported === false
      ? `File observation is not implemented on ${
          statusQuery.data?.platform ?? "this platform"
        }.`
      : observation?.runtime_available
        ? "Tracks files read by shell and process tools."
        : observation?.last_error ??
          "No observation attempt has run yet. Runtime availability is unknown.";
  const observationLabel = statusQuery.isLoading
    ? "Checking"
    : statusQuery.isError
      ? "Status unavailable"
      : observation?.platform_supported === false
        ? "Unsupported platform"
        : observation?.runtime_available
          ? "Runtime active"
          : observation?.last_error
            ? "Degraded attribution"
            : "Runtime unknown";
  const observationTone =
    statusQuery.isError ||
    observation?.platform_supported === false ||
    observation?.last_error
      ? "warning"
      : observation?.runtime_available
        ? "success"
        : "muted";

  const saveStatus = updateState.isLoading
    ? "saving"
    : updateState.isSuccess
      ? "saved"
      : updateState.isError
        ? "error"
        : "idle";

  const blockedProviders = data.destinations.filter(
    (destination) =>
      destination.kind === "provider" &&
      mcpServers.some(
        (server) =>
          !mcpAllowedForProvider(
            data.policy.tool_access,
            destination.id,
            server,
          ),
      ),
  );

  return (
    <SettingsSection
      title="Privacy & Access"
      description="Decide which files leave your machine, and what each destination is allowed to touch."
      width="wide"
    >
      {configError ? (
        <ErrorState
          aria-label="Privacy configuration error"
          title="Privacy configuration error"
          description={configError}
          variant="full"
        />
      ) : null}
      {saveError ? (
        <ErrorState
          title="Privacy policy was not saved"
          description={saveError}
        />
      ) : null}
      {data.has_project_overrides ? (
        <p className={styles.noteLine}>
          A project <code>.refact/privacy.yaml</code> tightens this policy
          further. You are editing the global policy here; project files can
          only narrow it, never widen it.
        </p>
      ) : null}

      <SettingsGroup
        title="Shell file observation"
        description={observationDescription}
      >
        <div className={`${styles.capabilityBanner} rf-enter`}>
          <Badge tone={observationTone} variant="soft">
            <StatusDot
              aria-hidden="true"
              status={
                observationTone === "warning"
                  ? "warning"
                  : observationTone === "success"
                    ? "success"
                    : "in_progress"
              }
            />
            {observationLabel}
          </Badge>
        </div>
      </SettingsGroup>

      <SettingsGroup
        title="What is sensitive"
        description="A zone is a set of files matched by glob patterns. A file belongs to the first zone listed here whose patterns match it, so put narrow zones above broad ones."
      >
        <div className={styles.groupContent}>
          <SaveStatus state={saveStatus} />
          <div className={styles.zoneStack}>
            <div className={styles.zoneCards}>
              {data.policy.zones.map((zone) => (
                <ZoneCard
                  key={zone.name}
                  matchCount={data.match_counts[zone.name] ?? 0}
                  removable={
                    data.policy.zones.length > 1 && !isCatchAllZone(zone)
                  }
                  saving={updateState.isLoading}
                  takenNames={data.policy.zones
                    .map((other) => other.name)
                    .filter((other) => other !== zone.name)}
                  zone={zone}
                  onChange={(patch) => updateZone(zone.name, patch)}
                  onRemove={() => removeZone(zone.name)}
                />
              ))}
            </div>
            <Button
              disabled={updateState.isLoading}
              leftIcon={Plus}
              size="sm"
              variant="soft"
              onClick={addZone}
            >
              Add zone
            </Button>
          </div>
        </div>
      </SettingsGroup>

      <SettingsGroup
        title="Blocked patterns"
        description="Blocked from every destination, including local models. Overrides every zone."
      >
        <div className={styles.blockedBox}>
          <PatternChips
            addLabel="Add blocked pattern"
            disabled={updateState.isLoading}
            emptyLabel="Nothing is globally blocked"
            patterns={data.policy.blocked}
            placeholder="e.g. id_rsa"
            onChange={(blocked) => void save({ ...data.policy, blocked })}
          />
        </div>
      </SettingsGroup>

      <SettingsGroup
        title="Who may touch what"
        description="Open a destination to choose which zones it may receive. Model providers can additionally be limited to a subset of MCP servers."
      >
        <div className={styles.groupContent}>
          <SaveStatus state={saveStatus} />
          <DestinationAccess
            destinations={data.destinations}
            matchCounts={data.match_counts}
            mcpServers={mcpServers}
            saving={updateState.isLoading}
            toolAccess={data.policy.tool_access}
            zones={data.policy.zones}
            onToggleMcp={handleMcpToggle}
            onToggleZone={handleZoneToggle}
          />
        </div>
        {blockedProviders.length > 0 ? (
          <p className={styles.noteLine}>
            {blockedProviders.length} provider
            {blockedProviders.length === 1 ? "" : "s"} cannot use every MCP
            server. Their blocked servers are hidden from the tool list and
            refused at call time.
          </p>
        ) : null}
      </SettingsGroup>

      <SettingsGroup title="Subagents">
        <SettingItem
          title="Subagent reports declassify"
          description="A subagent's summary may be sent onward even when it read restricted files. Turn off to re-check the parent's destination against everything the subagent read."
          control={
            <Switch
              aria-label="Subagent reports declassify"
              checked={data.policy.subagents.report_declassifies}
              disabled={updateState.isLoading}
              onCheckedChange={(report_declassifies) =>
                void save({
                  ...data.policy,
                  subagents: { report_declassifies },
                })
              }
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup
        title="Access matrix"
        description="Read-only overview of every zone against every destination."
      >
        <div className={styles.auditBox}>
          <Button
            aria-expanded={matrixOpen}
            size="sm"
            variant="ghost"
            onClick={() => setMatrixOpen((open) => !open)}
          >
            {matrixOpen ? "Hide matrix" : "Show matrix"}
          </Button>
          {matrixOpen ? (
            <AccessMatrix
              containerRef={matrixRef}
              destinations={data.destinations}
              matchCounts={data.match_counts}
              zones={data.policy.zones}
            />
          ) : null}
        </div>
      </SettingsGroup>
    </SettingsSection>
  );
}
