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
    undefined,
    {
      skip: provider.name !== "openrouter",
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

  const filteredModels = useMemo(() => {
    if (!modelsData?.models) return [];
    const query = searchQuery.trim().toLowerCase();
    if (!query) return modelsData.models;
    return modelsData.models.filter((model) => {
      const name = (model.display_name ?? model.id).toLowerCase();
      const id = model.id.toLowerCase();
      return name.includes(query) || id.includes(query);
    });
  }, [modelsData?.models, searchQuery]);

  const groupedByFamily = useMemo(() => {
    if (provider.name !== "openrouter") return null;
    const groups = new Map<string, typeof filteredModels>();

    filteredModels.forEach((model) => {
      const family = model.id.includes("/") ? model.id.split("/")[0] : "other";
      const entry = groups.get(family) ?? [];
      entry.push(model);
      groups.set(family, entry);
    });

    return Array.from(groups.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [filteredModels, provider.name]);

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

  const totalModels = modelsData.models.length;
  const enabledCount = modelsData.models.filter(
    (model) => model.enabled,
  ).length;

  return (
    <Flex direction="column" gap="3" mt="4">
      <Separator size="4" />

      <Flex align="center" justify="between" gap="3" wrap="wrap">
        <Flex align="center" gap="2" wrap="wrap">
          <Heading as="h3" size="3">
            Available Models
          </Heading>
          <Badge size="1" color="gray">
            {enabledCount}/{totalModels} enabled
          </Badge>
          <TextField.Root
            size="1"
            placeholder="Search models"
            value={searchQuery}
            onChange={(event) => setSearchQuery(event.target.value)}
            style={{ minWidth: 180 }}
          />
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

      {provider.name === "openrouter" && openRouterAccount?.data && (
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
              ? "No models available for this provider."
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
                      isReadonlyProvider={provider.readonly}
                      onEditCustomModel={handleOpenEditModal}
                    />
                  ))}
                </Flex>
              ))
            : filteredModels.map((model) => (
                <AvailableModelCard
                  key={model.id}
                  model={model}
                  providerName={provider.name}
                  isReadonlyProvider={provider.readonly}
                  onEditCustomModel={handleOpenEditModal}
                />
              ))}
        </Flex>
      )}

      <AddCustomModelModal
        providerName={provider.name}
        isOpen={isAddModalOpen}
        onClose={handleCloseModal}
        initialModel={editingModel}
      />
    </Flex>
  );
};
