import React from "react";
import { Copy } from "lucide-react";

import { Button } from "../../../components/ui";
import { ProviderForm } from "../ProviderForm";

import { getProviderName } from "../getProviderName";

import type { ProviderListItem } from "../../../services/refact";
import { DeletePopover } from "../../../components/DeletePopover";
import { useDeleteProviderMutation } from "../../../hooks/useProvidersQuery";
import { useAppDispatch } from "../../../hooks";
import { setInformation } from "../../Errors/informationSlice";
import { providersApi } from "../../../services/refact";
import styles from "./ProviderPreview.module.css";

export type ProviderPreviewProps = {
  configuredProviders: ProviderListItem[];
  currentProvider: ProviderListItem;
  handleSetCurrentProvider: (provider: ProviderListItem | null) => void;
  onDuplicateProvider?: (provider: ProviderListItem) => void;
};

export const ProviderPreview: React.FC<ProviderPreviewProps> = ({
  currentProvider,
  handleSetCurrentProvider,
  onDuplicateProvider,
}) => {
  const dispatch = useAppDispatch();
  const [deleteProvider, { isLoading: isDeletingProvider }] = useDeleteProviderMutation();

  const handleDeleteProvider = async (providerName: string) => {
    const response = await deleteProvider(providerName);
    if (response.error) return;
    dispatch(
      setInformation(
        `${getProviderName(currentProvider)}'s Provider configuration was deleted successfully`,
      ),
    );
    dispatch(providersApi.util.resetApiState());
    handleSetCurrentProvider(null);
  };

  return (
    <section className={styles.preview}>
      <div className={styles.headerRow}>
        <h2 className={styles.title}>{getProviderName(currentProvider)} Configuration</h2>
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
            itemName={getProviderName(currentProvider)}
            isDisabled={currentProvider.readonly}
            isDeleting={isDeletingProvider}
            deleteBy={currentProvider.name}
            handleDelete={(providerName: string) => void handleDeleteProvider(providerName)}
          />
        </div>
      </div>
      <ProviderForm currentProvider={currentProvider} />
    </section>
  );
};
