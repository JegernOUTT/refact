import { useCallback, useState } from "react";

import { useAppDispatch } from "../../../hooks";
import {
  useDeleteProviderMutation,
  useUpdateProviderMutation,
} from "../../../hooks/useProvidersQuery";

import { setInformation } from "../../Errors/informationSlice";
import { providersApi, ProviderListItem } from "../../../services/refact";

import { getProviderName } from "../getProviderName";

import type { ProviderFormValues } from "../ProviderForm/useProviderForm";

export function useProviderPreview(
  handleSetCurrentProvider: (provider: ProviderListItem | null) => void,
) {
  const dispatch = useAppDispatch();

  const [isSavingProvider, setIsSavingProvider] = useState(false);
  const [isDeletingProvider, setIsDeletingProvider] = useState(false);
  const [currentProviderName, setCurrentProviderName] = useState<string>("");

  const [updateProvider] = useUpdateProviderMutation();
  const [deleteProvider] = useDeleteProviderMutation();

  const handleSaveChanges = useCallback(
    async (updatedProviderData: ProviderFormValues, providerName: string) => {
      setIsSavingProvider(true);
      setCurrentProviderName(providerName);

      const { enabled, readonly: _readonly, ...settings } = updatedProviderData;
      const response = await updateProvider({
        providerName,
        settings: { ...settings, enabled },
      });

      if (response.error) {
        setIsSavingProvider(false);
        return;
      }
      const actions = [
        setInformation(`Provider ${providerName} updated successfully`),
        providersApi.util.invalidateTags([
          "PROVIDER",
          { type: "PROVIDERS", id: "LIST" },
        ]),
      ];
      actions.forEach((action) => dispatch(action));
      setIsSavingProvider(false);
    },
    [dispatch, updateProvider],
  );

  const handleDeleteProvider = useCallback(
    async (providerName: string) => {
      setIsDeletingProvider(true);
      const response = await deleteProvider(providerName);

      if (response.error) {
        setIsDeletingProvider(false);
        return;
      }

      const actions = [
        setInformation(
          `${getProviderName(
            providerName,
          )}'s Provider configuration was deleted successfully`,
        ),
        providersApi.util.resetApiState(),
      ];

      actions.forEach((action) => dispatch(action));
      handleSetCurrentProvider(null);
      setIsDeletingProvider(false);
    },
    [dispatch, deleteProvider, handleSetCurrentProvider],
  );

  const handleDiscardChanges = useCallback(() => {
    handleSetCurrentProvider(null);
  }, [handleSetCurrentProvider]);

  return {
    updateProvider,
    handleDeleteProvider,
    handleDiscardChanges,
    handleSaveChanges,
    isSavingProvider,
    isDeletingProvider,
    currentProviderName,
  };
}
