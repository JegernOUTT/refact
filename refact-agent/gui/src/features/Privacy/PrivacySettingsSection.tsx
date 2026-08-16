import { useCallback, useEffect, useMemo, useState } from "react";

import {
  Badge,
  Button,
  EditableTable,
  ErrorState,
  FieldSelect,
  LoadingState,
  SettingItem,
  StatusDot,
} from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  type PrivacyPolicy,
  type PrivacyShellBehavior,
  type PrivacyZone,
  useGetPrivacyPolicyQuery,
  useGetPrivacyStatusQuery,
  useUpdatePrivacyPolicyMutation,
} from "../../services/refact/privacy";
import { SettingsGroup, SettingsSection } from "../Settings/SettingsSection";
import { selectSelectedZoneName, setSelectedZone } from "./privacySlice";
import { ZoneGrid } from "./ZoneGrid";
import styles from "./PrivacySettingsSection.module.css";

type PatternRow = {
  pattern: string;
};

const SHELL_BEHAVIOR_OPTIONS = [
  { value: "withhold", label: "Withhold output" },
  { value: "ask", label: "Ask first" },
  { value: "deny", label: "Deny command" },
];

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

interface ZonePatternsEditorProps {
  zone: PrivacyZone;
  matchCount: number;
  saving: boolean;
  onSave: (patterns: string[]) => void;
}

function ZonePatternsEditor({
  matchCount,
  onSave,
  saving,
  zone,
}: ZonePatternsEditorProps) {
  const [rows, setRows] = useState<PatternRow[]>(() =>
    zone.patterns.map((pattern) => ({ pattern })),
  );

  useEffect(() => {
    setRows(zone.patterns.map((pattern) => ({ pattern })));
  }, [zone.name, zone.patterns]);

  const hasBlankPattern = rows.some((row) => row.pattern.trim().length === 0);
  const dirty =
    rows.length !== zone.patterns.length ||
    rows.some((row, index) => row.pattern !== zone.patterns[index]);

  return (
    <SettingItem
      layout="stack"
      title="File patterns"
      description={`${String(
        matchCount,
      )} workspace files currently match this zone.`}
      control={
        <div className={styles.patternEditor}>
          <EditableTable<PatternRow>
            addLabel="Add pattern"
            columns={[
              {
                id: "pattern",
                header: "Pattern",
                placeholder: "Glob pattern",
                getInputProps: () => ({ disabled: saving }),
              },
            ]}
            createRow={() => ({ pattern: "" })}
            emptyMessage="No patterns in this zone"
            removeLabel="Remove pattern"
            validate={({ value }) =>
              value.trim().length === 0 ? "Pattern is required" : null
            }
            value={rows}
            onChange={setRows}
          />
          <Button
            disabled={!dirty || hasBlankPattern}
            loading={saving}
            size="sm"
            variant="soft"
            onClick={() => onSave(rows.map((row) => row.pattern))}
          >
            Apply patterns
          </Button>
        </div>
      }
    />
  );
}

export function PrivacySettingsSection() {
  const dispatch = useAppDispatch();
  const selectedZoneName = useAppSelector(selectSelectedZoneName);
  const policyQuery = useGetPrivacyPolicyQuery(undefined);
  const statusQuery = useGetPrivacyStatusQuery(undefined);
  const [updatePolicy, updateState] = useUpdatePrivacyPolicyMutation();
  const [saveError, setSaveError] = useState<string | null>(null);

  const data = policyQuery.data;
  const selectedZone = useMemo(() => {
    if (!data) return null;
    const selected = data.policy.zones.find(
      (zone) => zone.name === selectedZoneName,
    );
    if (selected) return selected;
    return data.policy.zones[0];
  }, [data, selectedZoneName]);

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

  const handleDestinationToggle = useCallback(
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

  if (policyQuery.isLoading) {
    return (
      <SettingsSection
        title="Privacy"
        description="Control which files may reach each external destination."
      >
        <LoadingState label="Loading privacy policy" variant="full" />
      </SettingsSection>
    );
  }

  if (policyQuery.isError || !data) {
    return (
      <SettingsSection
        title="Privacy"
        description="Control which files may reach each external destination."
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

  return (
    <SettingsSection
      title="Privacy"
      description="Control which files may reach each provider, MCP server, subagent model, and completion service."
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

      <SettingsGroup title="Runtime protection">
        <SettingItem
          className={`${styles.capabilityBanner} rf-enter`}
          title="Shell file observation"
          description={
            statusQuery.isError
              ? "Observation capability could not be checked."
              : observation?.reason ??
                "Tracks files read by shell and process tools on this platform."
          }
          control={
            <Badge
              tone={
                statusQuery.isError || observation?.available === false
                  ? "warning"
                  : observation?.available
                    ? "success"
                    : "muted"
              }
              variant="soft"
            >
              <StatusDot
                aria-hidden="true"
                status={
                  statusQuery.isError || observation?.available === false
                    ? "warning"
                    : observation?.available
                      ? "success"
                      : "in_progress"
                }
              />
              {statusQuery.isLoading
                ? "Checking"
                : observation?.available
                  ? `Available on ${
                      statusQuery.data?.platform ?? "this platform"
                    }`
                  : "Degraded attribution"}
            </Badge>
          }
        />
      </SettingsGroup>

      <SettingsGroup title="Destination access">
        <SettingItem
          layout="stack"
          title="Zone permissions"
          description="Each destination column is independent. Select a zone name to edit its patterns."
          saveStatus={
            updateState.isLoading
              ? "saving"
              : updateState.isSuccess
                ? "saved"
                : updateState.isError
                  ? "error"
                  : "idle"
          }
          control={
            <ZoneGrid
              destinations={data.destinations}
              matchCounts={data.match_counts}
              saving={updateState.isLoading}
              selectedZoneName={selectedZone?.name ?? null}
              zones={data.policy.zones}
              onSelectZone={(zoneName) => dispatch(setSelectedZone(zoneName))}
              onToggle={handleDestinationToggle}
            />
          }
        />
      </SettingsGroup>

      {selectedZone ? (
        <SettingsGroup title={`Zone: ${selectedZone.name}`}>
          <ZonePatternsEditor
            matchCount={data.match_counts[selectedZone.name]}
            saving={updateState.isLoading}
            zone={selectedZone}
            onSave={(patterns) => updateZone(selectedZone.name, { patterns })}
          />
        </SettingsGroup>
      ) : null}

      <SettingsGroup title="Shell reads">
        {data.policy.zones.map((zone) => (
          <SettingItem
            className="rf-enter"
            key={zone.name}
            title={zone.name}
            description="Choose what happens when a shell command reads a file in this zone."
            control={
              <FieldSelect
                aria-label={`Shell read behavior for ${zone.name}`}
                disabled={updateState.isLoading}
                options={SHELL_BEHAVIOR_OPTIONS}
                value={zone.on_shell_read}
                onChange={(value) =>
                  updateZone(zone.name, {
                    on_shell_read: value as PrivacyShellBehavior,
                  })
                }
              />
            }
          />
        ))}
      </SettingsGroup>
    </SettingsSection>
  );
}
