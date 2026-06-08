import { useMemo, useState, type FC } from "react";
import classNames from "classnames";
import { Info, Plus } from "lucide-react";

import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  FieldText,
  Icon,
} from "../../../../components/ui";
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
import styles from "./ModelCard.module.css";

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
    { providerName: provider.name, useInstanceRoute: true },
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
      <ErrorState
        title="Failed to load models"
        description={`Failed to load models: ${errorMessage}`}
      />
    );
  }

  if (!isSuccess) {
    return (
      <EmptyState
        title="No model data available"
        description="Make sure the provider is properly configured."
      />
    );
  }

  const totalModels = providerModels.length;
  const enabledCount = providerModels.filter((model) => model.enabled).length;

  return (
    <section className={styles.modelsSection}>
      <div className={styles.modelsHeader}>
        <div className={styles.modelsHeaderCopy}>
          <h3 className={styles.modelsTitle}>Available Models</h3>
          <Badge tone="muted">
            {isCustomProvider && totalModels === 0
              ? "None"
              : `${enabledCount}/${totalModels} enabled`}
          </Badge>
          {totalModels > 0 ? (
            <FieldText
              className={styles.searchInput}
              placeholder="Search models"
              value={searchQuery}
              onChange={setSearchQuery}
            />
          ) : null}
        </div>

        {!provider.readonly ? (
          <Button
            size="sm"
            variant="soft"
            leftIcon={Plus}
            onClick={handleOpenCreateModal}
          >
            Add Custom Model
          </Button>
        ) : null}
      </div>

      {modelsData.error ? (
        <div className={styles.notice} role="status">
          <Icon icon={Info} size="sm" tone="warning" />
          {modelsData.error}
        </div>
      ) : null}

      {baseProvider === "openrouter" && openRouterAccount?.data ? (
        <div className={styles.notice} role="status">
          <Icon icon={Info} size="sm" tone="accent" />
          OpenRouter balance:{" "}
          {openRouterAccount.data.remaining?.toFixed(2) ?? "0.00"}
          {" / "}
          {openRouterAccount.data.limit?.toFixed(2) ?? "0.00"} USD
          {openRouterAccount.data.key_label
            ? ` · Key: ${openRouterAccount.data.key_label}`
            : ""}
        </div>
      ) : null}

      {filteredModels.length === 0 ? (
        <div className={styles.emptyCopy}>
          <span>
            {totalModels === 0
              ? isCustomProvider
                ? "No custom models configured."
                : "No models available for this provider."
              : "No models match your search."}
          </span>
          {!provider.readonly ? (
            <span>Click &quot;Add Custom Model&quot; to define your own.</span>
          ) : null}
        </div>
      ) : (
        <div className={classNames(styles.modelsList, "rf-stagger")}>
          {groupedByFamily
            ? groupedByFamily.map(([family, group]) => (
                <div key={family} className={styles.modelFamilyGroup}>
                  <span className={styles.familyLabel}>
                    {toPascalCase(family)} · {group.length}
                  </span>
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
                </div>
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
        </div>
      )}

      <AddCustomModelModal
        providerName={provider.name}
        isOpen={isAddModalOpen}
        onClose={handleCloseModal}
        initialModel={editingModel}
        isEditingCustomModel={editingModel?.is_custom ?? false}
      />
    </section>
  );
};
