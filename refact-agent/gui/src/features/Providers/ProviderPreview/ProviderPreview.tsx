import React from "react";
import { ArrowLeft, Copy } from "lucide-react";

import { Button } from "../../../components/ui";
import { ProviderForm } from "../ProviderForm";

import { getProviderName } from "../getProviderName";

import type { ProviderListItem } from "../../../services/refact";
import { DeletePopover } from "../../../components/DeletePopover";
import { useDeleteProviderMutation } from "../../../hooks/useProvidersQuery";
import { useAppDispatch } from "../../../hooks";
import { setInformation } from "../../Errors/informationSlice";
import { providersApi } from "../../../services/refact";
import { SettingsSection } from "../../Settings/SettingsSection";
import styles from "./ProviderPreview.module.css";

export type ProviderPreviewProps = {
  configuredProviders: ProviderListItem[];
  currentProvider: ProviderListItem;
  handleSetCurrentProvider: (provider: ProviderListItem | null) => void;
  onDuplicateProvider?: (provider: ProviderListItem) => void;
  onBack?: () => void;
  sectioned?: boolean;
};

export const ProviderPreview: React.FC<ProviderPreviewProps> = ({
  currentProvider,
  handleSetCurrentProvider,
  onDuplicateProvider,
  onBack,
  sectioned = false,
}) => {
  const dispatch = useAppDispatch();
  const [deleteProvider, { isLoading: isDeletingProvider }] =
    useDeleteProviderMutation();
  const providerName = getProviderName(currentProvider);

  const handleDeleteProvider = async (providerNameToDelete: string) => {
    const response = await deleteProvider(providerNameToDelete);
    if (response.error) return;
    dispatch(
      setInformation(
        `${providerName}'s Provider configuration was deleted successfully`,
      ),
    );
    dispatch(providersApi.util.resetApiState());
    handleSetCurrentProvider(null);
  };

  const actions = (
    <div className={styles.actions}>
      {onDuplicateProvider ? (
        <Button
          type="button"
          size="md"
          variant="soft"
          leftIcon={Copy}
          onClick={() => onDuplicateProvider(currentProvider)}
        >
          Duplicate instance
        </Button>
      ) : null}
      <DeletePopover
        itemName={providerName}
        isDisabled={currentProvider.readonly}
        isDeleting={isDeletingProvider}
        deleteBy={currentProvider.name}
        handleDelete={(providerNameToDelete: string) =>
          void handleDeleteProvider(providerNameToDelete)
        }
      />
    </div>
  );

  if (sectioned) {
    return (
      <SettingsSection
        title={`${providerName} Configuration`}
        description="Edit credentials, model availability, and default model routing for this provider instance."
        width="wide"
        subNav={
          onBack ? (
            <Button variant="ghost" leftIcon={ArrowLeft} onClick={onBack}>
              Back
            </Button>
          ) : null
        }
        actions={actions}
      >
        <ProviderForm currentProvider={currentProvider} />
      </SettingsSection>
    );
  }

  return (
    <section className={styles.preview}>
      <div className={styles.headerRow}>
        <h2 className={styles.title}>{providerName} Configuration</h2>
        {actions}
      </div>
      <ProviderForm currentProvider={currentProvider} />
    </section>
  );
};
