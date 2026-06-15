import { useCallback, useMemo, type FC } from "react";
import classNames from "classnames";
import { MoreVertical } from "lucide-react";

import {
  Badge,
  IconButton,
  Menu,
  Surface,
  Tooltip,
} from "../../../../components/ui";
import { ModelCardPopup } from "./components/ModelCardPopup";
import {
  CapabilityIcons,
  ContextWindowIcon,
  ModelDetailIcon,
  PricingIcon,
} from "./components/CapabilityIcons";
import { useModelDialogState } from "./hooks/useModelDialogState";

import type { ModelType } from "../../../../services/refact";
import type { UiModel } from "./utils/groupModelsWithPricing";

import styles from "./ModelCard.module.css";
import { useEventsBusForIDE } from "../../../../hooks";

export type ModelCardProps = {
  model: UiModel;
  providerName: string;
  modelType: ModelType;
  isReadonlyProvider: boolean;
  currentModelNames: string[];
};

export const ModelCard: FC<ModelCardProps> = ({
  model,
  modelType,
  providerName,
  isReadonlyProvider,
  currentModelNames,
}) => {
  const { enabled, name, removable, user_configured } = model;
  const {
    isOpen: dialogOpen,
    setIsOpen: setDialogOpen,
    dropdownOpen,
    setDropdownOpen,
    openDialogSafely,
    isSavingModel,
    handleToggleModelEnabledState,
    handleRemoveModel,
    handleResetModel,
    handleSaveModel,
    handleUpdateModel,
  } = useModelDialogState({
    initialState: false,
    modelType,
    providerName,
  });

  const { setCodeCompletionModel } = useEventsBusForIDE();

  const handleSetCompletionModelForIDE = useCallback(() => {
    const formattedModelName = `${providerName}/${model.name}`;
    setCodeCompletionModel(formattedModelName);
  }, [model, providerName, setCodeCompletionModel]);

  const dropdownOptions = useMemo(() => {
    const shouldOptionsBeDisabled = isReadonlyProvider || isSavingModel;
    return [
      {
        label: "Edit model's settings",
        onClick: openDialogSafely,
        visible: !shouldOptionsBeDisabled,
      },
      {
        label: enabled ? "Disable model" : "Enable model",
        onClick: () => void handleToggleModelEnabledState(model),
        visible: !shouldOptionsBeDisabled,
      },
      {
        label: "Reset model",
        onClick: () => void handleResetModel(model),
        visible: !removable && user_configured,
      },
      {
        label: "Remove model",
        onClick: () => void handleRemoveModel({ model }),
        visible: removable,
      },
      {
        label: "Use as completion model in IDE",
        onClick: handleSetCompletionModelForIDE,
        visible: modelType === "completion",
      },
    ];
  }, [
    isReadonlyProvider,
    isSavingModel,
    enabled,
    removable,
    user_configured,
    model,
    modelType,
    openDialogSafely,
    handleToggleModelEnabledState,
    handleResetModel,
    handleRemoveModel,
    handleSetCompletionModelForIDE,
  ]);

  const visibleDropdownOptions = useMemo(() => {
    return dropdownOptions.filter((option) => option.visible);
  }, [dropdownOptions]);

  return (
    <Surface
      variant="glass"
      animated="rise"
      className={classNames(styles.modelCard, {
        [styles.disabledCard]: isSavingModel,
      })}
    >
      {dialogOpen ? (
        <ModelCardPopup
          minifiedModel={model}
          isOpen={dialogOpen}
          isSaving={isSavingModel}
          setIsOpen={setDialogOpen}
          modelName={name}
          modelType={modelType}
          providerName={providerName}
          onSave={handleSaveModel}
          onUpdate={handleUpdateModel}
          isRemovable={removable}
          currentModelNames={currentModelNames}
        />
      ) : null}

      <div className={styles.modelHeader}>
        <div className={styles.modelCopy}>
          <div className={styles.modelTitleRow}>
            <span className={styles.modelName}>{name}</span>
            <Badge tone={enabled ? "success" : "muted"}>
              {enabled ? "Active" : "Inactive"}
            </Badge>
          </div>

          <div className={styles.modelMetaRow}>
            {model.pricingLabel ? (
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <span>
                    <ModelDetailIcon icon={<PricingIcon />}>
                      {model.pricingLabel}
                    </ModelDetailIcon>
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Content>
                  Price per 1M tokens (prompt/output)
                </Tooltip.Content>
              </Tooltip>
            ) : null}
            {model.nCtxLabel ? (
              <Tooltip>
                <Tooltip.Trigger asChild>
                  <span>
                    <ModelDetailIcon icon={<ContextWindowIcon />}>
                      {model.nCtxLabel}
                    </ModelDetailIcon>
                  </span>
                </Tooltip.Trigger>
                <Tooltip.Content>
                  Context window: {model.nCtx?.toLocaleString()} tokens
                </Tooltip.Content>
              </Tooltip>
            ) : null}
            {model.capabilities ? (
              <CapabilityIcons capabilities={model.capabilities} size="1" />
            ) : null}
          </div>
        </div>

        {visibleDropdownOptions.length > 0 ? (
          <Menu open={dropdownOpen} onOpenChange={setDropdownOpen}>
            <Menu.Trigger asChild>
              <IconButton
                size="sm"
                variant="ghost"
                aria-label="Model actions"
                icon={MoreVertical}
              />
            </Menu.Trigger>
            <Menu.Content side="bottom" align="end" maxWidth="260px">
              {visibleDropdownOptions.map(({ label, onClick }) => (
                <Menu.Item key={label} onClick={onClick} title={label}>
                  {label}
                </Menu.Item>
              ))}
            </Menu.Content>
          </Menu>
        ) : null}
      </div>
    </Surface>
  );
};
