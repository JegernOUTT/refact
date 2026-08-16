import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  Button,
  Dialog,
  FieldSelect,
  FieldStack,
  FieldText,
} from "../../../components/ui";
import { Checkbox } from "../../../components/Checkbox";

import type { ProviderListItem } from "../../../services/refact";
import {
  type CreateProviderInstanceRequest,
  providersApi,
  useUpdateProviderMutation,
} from "../../../services/refact";
import {
  type PrivacyPolicy,
  useGetPrivacyPolicyQuery,
  useUpdatePrivacyPolicyMutation,
} from "../../../services/refact/privacy";
import { useAppDispatch } from "../../../hooks";
import {
  nextInstanceId,
  providerBaseOptions,
  providerInstanceDisplayName,
  validateProviderInstanceId,
} from "./providerInstanceUtils";

import styles from "./AddProviderInstanceModal.module.css";

export type AddProviderInstanceModalProps = {
  isOpen: boolean;
  configuredProviders: ProviderListItem[];
  initialBaseProvider: string | null;
  onOpenChange: (open: boolean) => void;
  onCreated: (provider: ProviderListItem) => void;
};

function getErrorMessage(error: unknown) {
  if (typeof error === "object" && error !== null) {
    const record = error as Record<string, unknown>;
    const data = record.data;
    if (typeof data === "object" && data !== null) {
      const dataRecord = data as Record<string, unknown>;
      if (typeof dataRecord.detail === "string") return dataRecord.detail;
      if (typeof dataRecord.error === "string") return dataRecord.error;
    }
    if (typeof data === "string") return data;
    if (typeof record.error === "string") return record.error;
    if (typeof record.message === "string") return record.message;
  }
  return "Failed to create the provider or save its privacy zones.";
}

function applyProviderTrust(
  policy: PrivacyPolicy,
  providerId: string,
  selectedZoneNames: string[],
  knownDestinationIds: string[],
): PrivacyPolicy {
  const selected = new Set(selectedZoneNames);

  return {
    ...policy,
    zones: policy.zones.map((zone) => {
      const allowed = selected.has(zone.name);
      if (zone.send_to.includes("*")) {
        return allowed
          ? zone
          : {
              ...zone,
              send_to: Array.from(
                new Set(knownDestinationIds.filter((id) => id !== providerId)),
              ),
            };
      }

      return {
        ...zone,
        send_to: allowed
          ? Array.from(new Set([...zone.send_to, providerId]))
          : zone.send_to.filter((id) => id !== providerId),
      };
    }),
  };
}

export const AddProviderInstanceModal: React.FC<
  AddProviderInstanceModalProps
> = ({
  isOpen,
  configuredProviders,
  initialBaseProvider,
  onOpenChange,
  onCreated,
}) => {
  const dispatch = useAppDispatch();
  const [updateProvider, { isLoading }] = useUpdateProviderMutation();
  const privacyPolicyQuery = useGetPrivacyPolicyQuery(undefined, {
    skip: !isOpen,
  });
  const [updatePrivacyPolicy, privacyUpdateState] =
    useUpdatePrivacyPolicyMutation();
  const isSaving = isLoading || privacyUpdateState.isLoading;
  const providerNames = useMemo(
    () => configuredProviders.map((provider) => provider.name),
    [configuredProviders],
  );
  const baseOptions = useMemo(
    () => providerBaseOptions(configuredProviders),
    [configuredProviders],
  );
  const defaultBaseProvider = useMemo(() => {
    if (
      initialBaseProvider &&
      baseOptions.some((option) => option.id === initialBaseProvider)
    ) {
      return initialBaseProvider;
    }
    return baseOptions[0]?.id ?? "";
  }, [baseOptions, initialBaseProvider]);

  const [baseProvider, setBaseProvider] = useState(defaultBaseProvider);
  const [instanceId, setInstanceId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [idTouched, setIdTouched] = useState(false);
  const [displayNameTouched, setDisplayNameTouched] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [selectedZoneNames, setSelectedZoneNames] = useState<string[]>([]);

  useEffect(() => {
    if (!isOpen || !defaultBaseProvider) return;
    const nextId = nextInstanceId(defaultBaseProvider, providerNames);
    setBaseProvider(defaultBaseProvider);
    setInstanceId(nextId);
    setDisplayName(providerInstanceDisplayName(defaultBaseProvider, nextId));
    setIdTouched(false);
    setDisplayNameTouched(false);
    setLocalError(null);
  }, [defaultBaseProvider, isOpen, providerNames]);

  useEffect(() => {
    if (!isOpen || !privacyPolicyQuery.data) return;
    setSelectedZoneNames(
      privacyPolicyQuery.data.policy.zones
        .filter((zone) => zone.name === "normal")
        .map((zone) => zone.name),
    );
  }, [isOpen, privacyPolicyQuery.data]);

  const idValidation = useMemo(
    () => validateProviderInstanceId(instanceId, providerNames),
    [instanceId, providerNames],
  );
  const displayNameValidation = displayName.trim()
    ? null
    : "Display name is required.";
  const canSubmit =
    Boolean(baseProvider) &&
    !idValidation &&
    !displayNameValidation &&
    !isSaving &&
    Boolean(privacyPolicyQuery.data) &&
    !privacyPolicyQuery.isLoading &&
    !privacyPolicyQuery.isError;

  const handleZoneChange = useCallback((zoneName: string, checked: boolean) => {
    setSelectedZoneNames((current) =>
      checked
        ? Array.from(new Set([...current, zoneName]))
        : current.filter((name) => name !== zoneName),
    );
    setLocalError(null);
  }, []);

  const handleBaseProviderChange = useCallback(
    (nextBaseProvider: string) => {
      const generatedInstanceId = nextInstanceId(
        nextBaseProvider,
        providerNames,
      );
      const nextInstanceIdValue = idTouched ? instanceId : generatedInstanceId;
      setBaseProvider(nextBaseProvider);
      if (!idTouched) setInstanceId(generatedInstanceId);
      if (!displayNameTouched) {
        setDisplayName(
          providerInstanceDisplayName(nextBaseProvider, nextInstanceIdValue),
        );
      }
      setLocalError(null);
    },
    [displayNameTouched, idTouched, instanceId, providerNames],
  );

  const handleInstanceIdChange = useCallback(
    (nextId: string) => {
      setInstanceId(nextId);
      setIdTouched(true);
      if (!displayNameTouched) {
        setDisplayName(providerInstanceDisplayName(baseProvider, nextId));
      }
      setLocalError(null);
    },
    [baseProvider, displayNameTouched],
  );

  const handleDisplayNameChange = useCallback((nextDisplayName: string) => {
    setDisplayName(nextDisplayName);
    setDisplayNameTouched(true);
    setLocalError(null);
  }, []);

  const handleSubmit = useCallback(async () => {
    const trimmedInstanceId = instanceId.trim();
    const trimmedDisplayName = displayName.trim();
    const validation =
      validateProviderInstanceId(trimmedInstanceId, providerNames) ??
      (trimmedDisplayName ? null : "Display name is required.");
    if (!baseProvider || validation) {
      setLocalError(validation ?? "Select a base provider.");
      return;
    }
    if (!privacyPolicyQuery.data) {
      setLocalError("Privacy zones are unavailable.");
      return;
    }

    try {
      const settings: CreateProviderInstanceRequest = {
        base_provider: baseProvider,
        display_name: trimmedDisplayName,
        enabled: false,
      };
      await updateProvider({
        providerName: trimmedInstanceId,
        settings,
      }).unwrap();
      await updatePrivacyPolicy(
        applyProviderTrust(
          privacyPolicyQuery.data.policy,
          trimmedInstanceId,
          selectedZoneNames,
          [
            ...privacyPolicyQuery.data.destinations.map(
              (destination) => destination.id,
            ),
            ...configuredProviders.map((provider) => provider.name),
          ],
        ),
      ).unwrap();
      dispatch(
        providersApi.util.invalidateTags([
          { type: "PROVIDERS", id: "LIST" },
          { type: "PROVIDER", id: trimmedInstanceId },
          { type: "PROVIDER_MODELS", id: trimmedInstanceId },
          { type: "AVAILABLE_MODELS", id: trimmedInstanceId },
        ]),
      );
      onOpenChange(false);
      onCreated({
        name: trimmedInstanceId,
        base_provider: baseProvider,
        display_name: trimmedDisplayName,
        enabled: false,
        readonly: false,
        has_credentials: false,
        status: "not_configured",
        model_count: 0,
      });
    } catch (error) {
      setLocalError(getErrorMessage(error));
    }
  }, [
    baseProvider,
    configuredProviders,
    dispatch,
    displayName,
    instanceId,
    onCreated,
    onOpenChange,
    privacyPolicyQuery.data,
    providerNames,
    selectedZoneNames,
    updatePrivacyPolicy,
    updateProvider,
  ]);

  const handleOpenChange = useCallback(
    (open: boolean) => {
      if (!open && isSaving) return;
      onOpenChange(open);
    },
    [isSaving, onOpenChange],
  );

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      <Dialog.Content maxWidth="min(420px, calc(100vw - 2 * var(--rf-space-3)))">
        <Dialog.Title>Add provider instance</Dialog.Title>
        <Dialog.Description>
          Create a blank provider configuration and decide what it may see.
        </Dialog.Description>

        <div className={styles.formStack}>
          {baseOptions.length > 0 ? (
            <FieldStack
              label="Base provider"
              htmlFor="provider-instance-base"
              control={
                <FieldSelect
                  value={baseProvider}
                  options={baseOptions.map((option) => ({
                    value: option.id,
                    label: option.label,
                  }))}
                  onChange={handleBaseProviderChange}
                  disabled={isSaving}
                />
              }
            />
          ) : (
            <div className={styles.errorText}>
              No user-creatable base providers are available.
            </div>
          )}

          <FieldStack
            label="Instance id"
            htmlFor="provider-instance-id"
            helper={idValidation ?? "Use this id as the model prefix."}
            error={idValidation}
            control={
              <FieldText
                id="provider-instance-id"
                value={instanceId}
                onChange={handleInstanceIdChange}
                disabled={isSaving || baseOptions.length === 0}
                placeholder="openai_2"
              />
            }
          />

          <FieldStack
            label="Display name"
            htmlFor="provider-display-name"
            error={displayNameValidation}
            control={
              <FieldText
                id="provider-display-name"
                value={displayName}
                onChange={handleDisplayNameChange}
                disabled={isSaving || baseOptions.length === 0}
                placeholder="OpenAI 2"
              />
            }
          />

          <fieldset className={styles.trustFieldset}>
            <legend>What may this provider see?</legend>
            <div className={styles.helperText}>
              Normal files are selected by default. Choose any additional
              privacy zones you trust this provider to receive.
            </div>
            {privacyPolicyQuery.isLoading ? (
              <div className={styles.helperText}>Loading privacy zones...</div>
            ) : privacyPolicyQuery.isError || !privacyPolicyQuery.data ? (
              <div className={styles.errorText}>
                Privacy zones are unavailable.
              </div>
            ) : (
              <div className={styles.zoneList}>
                {privacyPolicyQuery.data.policy.zones.map((zone) => (
                  <Checkbox
                    key={zone.name}
                    checked={selectedZoneNames.includes(zone.name)}
                    disabled={isSaving}
                    onCheckedChange={(checked) =>
                      handleZoneChange(zone.name, checked === true)
                    }
                  >
                    {zone.name}
                  </Checkbox>
                ))}
              </div>
            )}
          </fieldset>

          {localError ? (
            <div className={styles.errorText}>{localError}</div>
          ) : null}
        </div>

        <div className={styles.dialogActions}>
          <Dialog.Close asChild>
            <Button variant="soft" disabled={isSaving}>
              Cancel
            </Button>
          </Dialog.Close>
          <Button
            variant="primary"
            onClick={() => void handleSubmit()}
            disabled={!canSubmit}
          >
            {isSaving ? "Creating..." : "Create instance"}
          </Button>
        </div>
      </Dialog.Content>
    </Dialog>
  );
};
