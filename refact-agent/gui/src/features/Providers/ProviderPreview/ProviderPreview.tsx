import React from "react";
import { Flex, Heading } from "@radix-ui/themes";

import { ProviderForm } from "../ProviderForm";

import { getProviderName } from "../getProviderName";

import type { ProviderListItem } from "../../../services/refact";
import { DeletePopover } from "../../../components/DeletePopover";
import { useDeleteProviderMutation } from "../../../hooks/useProvidersQuery";
import { useAppDispatch } from "../../../hooks";
import { setInformation } from "../../Errors/informationSlice";
import { providersApi } from "../../../services/refact";

export type ProviderPreviewProps = {
  configuredProviders: ProviderListItem[];
  currentProvider: ProviderListItem;
  handleSetCurrentProvider: (provider: ProviderListItem | null) => void;
};

export const ProviderPreview: React.FC<ProviderPreviewProps> = ({
  currentProvider,
  handleSetCurrentProvider,
}) => {
  const dispatch = useAppDispatch();
  const [deleteProvider, { isLoading: isDeletingProvider }] =
    useDeleteProviderMutation();

  const handleDeleteProvider = async (providerName: string) => {
    const response = await deleteProvider(providerName);
    if (response.error) return;
    dispatch(
      setInformation(
        `${getProviderName(
          currentProvider,
        )}'s Provider configuration was deleted successfully`,
      ),
    );
    dispatch(providersApi.util.resetApiState());
    handleSetCurrentProvider(null);
  };

  return (
    <Flex direction="column" align="start" minHeight="100%">
      <Flex justify="between" align="center" width="100%" mb="4">
        <Heading as="h2" size="3">
          {getProviderName(currentProvider)} Configuration
        </Heading>
        <DeletePopover
          itemName={getProviderName(currentProvider)}
          isDisabled={currentProvider.readonly}
          isDeleting={isDeletingProvider}
          deleteBy={currentProvider.name}
          handleDelete={(providerName: string) =>
            void handleDeleteProvider(providerName)
          }
        />
      </Flex>
      <ProviderForm currentProvider={currentProvider} />
    </Flex>
  );
};
