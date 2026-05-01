import { useMemo, useState, type FC } from "react";
import {
  Badge,
  Button,
  Callout,
  Flex,
  Heading,
  Separator,
  Text,
  TextField,
} from "@radix-ui/themes";
import { PlusIcon, InfoCircledIcon } from "@radix-ui/react-icons";

import type { ProviderListItem } from "../../../../services/refact";
import {
  useGetAvailableModelsQuery,
  useGetOpenRouterAccountInfoQuery,
  type AvailableModel,
} from "../../../../services/refact";
import { toPascalCase } from "../../../../utils/toPascalCase";

import { Spinner } from "../../../../components/Spinner";
import { AvailableModelCard } from "./AvailableModelCard";
import { AddCustomModelModal } from "./AddCustomModelModal";

export type ProviderModelsListProps = {
  provider: ProviderListItem;
};

export const ProviderModelsList: FC<ProviderModelsListProps> = ({
  provider,
}) => {
  const [searchQuery, setSearchQuery] = useState("");
  const baseProvider = provider.base_provider;
  const isCustomProvider = baseProvider === "custom";
  const {
    data: modelsData,
    isSuccess,
    isLoading,
    isError,
    error,
  } = useGetAvailableModelsQuery({ providerName: provider.name });

  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [editingModel, setEditingModel] = useState<
    AvailableModel | undefined
  >();
  const { data: openRouterAccount } = useGetOpenRouterAccountInfoQuery(
    { providerName: provider.name, useInstanceRoute: false },
    {
      skip: baseProvider !== "openrouter",
    },
  );

  const handleOpenCreateModal = () => {
    setEditingModel(undefined);
    setIsAddModalOpen(true);
  };

  const handleOpenEditModal = (model: AvailableModel) => {
    setEditingModel(model);
    setIsAddModalOpen(true);
  };

  const handleCloseModal = () => {
    setIsAddModalOpen(false);
    setEditingModel(undefined);
  };

  const providerModels = useMemo(() => {
    if (!modelsData?.models) return [];
    if (!isCustomProvider) return modelsData.models;

    return modelsData.models.filter((model) => model.is_custom);
  }, [isCustomProvider, modelsData?.models]);

  const filteredModels = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    if (!query) return providerModels;
    return providerModels.filter((model) => {
      const name = (model.display_name ?? model.id).toLowerCase();
      const id = model.id.toLowerCase();
      return name.includes(query) || id.includes(query);
    });
  }, [providerModels, searchQuery]);

  const groupedByFamily = useMemo(() => {
    if (baseProvider !== "openrouter") return null;
    const groups = new Map<string, typeof filteredModels>();

    filteredModels.forEach((model) => {
      const family = model.id.includes("/") ? model.id.split("/")[0] : "other";
      const entry = groups.get(family) ?? [];
      entry.push(model);
      groups.set(family, entry);
    });

    return Array.from(groups.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [baseProvider, filteredModels]);

  if (isLoading) return <Spinner spinning />;

  if (isError) {
    const err = error as
      | { status?: unknown; data?: { detail?: unknown } }
      | undefined;
    const errorMessage = err?.status
      ? `${String(err.status)}: ${
          err.data?.detail ? String(err.data.detail) : "Unknown error"
        }`
      : "Failed to load models";

    return (
      <Callout.Root color="red">
        <Callout.Icon>
          <InfoCircledIcon />
        </Callout.Icon>
        <Callout.Text>Failed to load models: {errorMessage}</Callout.Text>
      </Callout.Root>
    );
  }

  if (!isSuccess) {
    return (
      <Callout.Root color="orange">
        <Callout.Icon>
          <InfoCircledIcon />
        </Callout.Icon>
        <Callout.Text>
          No model data available. Make sure the provider is properly
          configured.
        </Callout.Text>
      </Callout.Root>
    );
  }

  const totalModels = providerModels.length;
  const enabledCount = providerModels.filter((model) => model.enabled).length;

  return (
    <Flex direction="column" gap="3" mt="4" flexShrink="0">
      <Separator size="4" />

      <Flex align="center" justify="between" gap="3" wrap="wrap">
        <Flex align="center" gap="2" wrap="wrap">
          <Heading as="h3" size="3">
            Available Models
          </Heading>
          <Badge size="1" color="gray">
            {isCustomProvider && totalModels === 0
              ? "None"
              : `${enabledCount}/${totalModels} enabled`}
          </Badge>
          {totalModels > 0 && (
            <TextField.Root
              size="1"
              placeholder="Search models"
              value={searchQuery}
              onChange={(event) => setSearchQuery(event.target.value)}
              style={{ minWidth: 180 }}
            />
          )}
        </Flex>

        {!provider.readonly && (
          <Button size="1" variant="soft" onClick={handleOpenCreateModal}>
            <PlusIcon /> Add Custom Model
          </Button>
        )}
      </Flex>

      {modelsData.error && (
        <Callout.Root color="orange" size="1">
          <Callout.Icon>
            <InfoCircledIcon />
          </Callout.Icon>
          <Callout.Text size="1">{modelsData.error}</Callout.Text>
        </Callout.Root>
      )}

      {baseProvider === "openrouter" && openRouterAccount?.data && (
        <Callout.Root color="blue" size="1">
          <Callout.Icon>
            <InfoCircledIcon />
          </Callout.Icon>
          <Callout.Text size="1">
            OpenRouter balance:{" "}
            {openRouterAccount.data.remaining?.toFixed(2) ?? "0.00"}
            {" / "}
            {openRouterAccount.data.limit?.toFixed(2) ?? "0.00"} USD
            {openRouterAccount.data.key_label
              ? ` · Key: ${openRouterAccount.data.key_label}`
              : ""}
          </Callout.Text>
        </Callout.Root>
      )}

      {filteredModels.length === 0 ? (
        <Flex direction="column" align="center" gap="2" py="4">
          <Text as="span" size="2" color="gray">
            {totalModels === 0
              ? isCustomProvider
                ? "No custom models configured."
                : "No models available for this provider."
              : "No models match your search."}
          </Text>
          {!provider.readonly && (
            <Text as="span" size="1" color="gray">
              Click &quot;Add Custom Model&quot; to define your own.
            </Text>
          )}
        </Flex>
      ) : (
        <Flex direction="column" gap="2">
          {groupedByFamily
            ? groupedByFamily.map(([family, group]) => (
                <Flex key={family} direction="column" gap="2">
                  <Text as="span" size="1" color="gray" weight="medium" mt="2">
                    {toPascalCase(family)} · {group.length}
                  </Text>
                  {group.map((model) => (
                    <AvailableModelCard
                      key={model.id}
                      model={model}
                      providerName={provider.name}
                      baseProvider={baseProvider}
                      isReadonlyProvider={provider.readonly}
                      onEditModel={handleOpenEditModal}
                    />
                  ))}
                </Flex>
              ))
            : filteredModels.map((model) => (
                <AvailableModelCard
                  key={model.id}
                  model={model}
                  providerName={provider.name}
                  baseProvider={baseProvider}
                  isReadonlyProvider={provider.readonly}
                  onEditModel={handleOpenEditModal}
                />
              ))}
        </Flex>
      )}

      <AddCustomModelModal
        providerName={provider.name}
        isOpen={isAddModalOpen}
        onClose={handleCloseModal}
        initialModel={editingModel}
        isEditingCustomModel={editingModel?.is_custom ?? false}
      />
    </Flex>
  );
};
