import React, { useCallback, useState } from "react";
import { ArrowLeft } from "lucide-react";
import classNames from "classnames";

import { Button } from "../../../components/ui";
import { ConfiguredProvidersView } from "./ConfiguredProvidersView";
import { AddProviderInstanceModal } from "./AddProviderInstanceModal";

import type { ProviderListItem } from "../../../services/refact";
import { ProviderPreview } from "../ProviderPreview";
import { ErrorCallout, InformationCallout } from "../../../components/Callout/Callout";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { clearError, getErrorMessage } from "../../Errors/errorsSlice";
import { clearInformation, getInformationMessage } from "../../Errors/informationSlice";

import styles from "./ProvidersView.module.css";
import { selectConfig } from "../../Config/configSlice";

export type ProvidersViewProps = {
  configuredProviders: ProviderListItem[];
  backFromProviders: () => void;
};

export const ProvidersView: React.FC<ProvidersViewProps> = ({
  configuredProviders,
  backFromProviders,
}) => {
  const dispatch = useAppDispatch();

  const currentHost = useAppSelector(selectConfig).host;
  const globalError = useAppSelector(getErrorMessage);
  const information = useAppSelector(getInformationMessage);

  const [currentProvider, setCurrentProvider] = useState<ProviderListItem | null>(null);
  const [instanceModalOpen, setInstanceModalOpen] = useState(false);
  const [initialBaseProvider, setInitialBaseProvider] = useState<string | null>(null);
  const handleSetCurrentProvider = useCallback((provider: ProviderListItem | null) => {
    setCurrentProvider(provider);
  }, []);

  const handleAddInstance = useCallback(() => {
    setInitialBaseProvider(null);
    setInstanceModalOpen(true);
  }, []);

  const handleDuplicateProvider = useCallback((provider: ProviderListItem) => {
    setInitialBaseProvider(provider.base_provider);
    setInstanceModalOpen(true);
  }, []);

  const handleInstanceCreated = useCallback((provider: ProviderListItem) => {
    setCurrentProvider(provider);
  }, []);

  const handleBackClick = useCallback(() => {
    if (currentProvider) {
      setCurrentProvider(null);
    } else {
      backFromProviders();
    }
  }, [currentProvider, backFromProviders]);

  return (
    <div className={styles.view}>
      <div className={styles.backRow}>
        <Button variant="ghost" leftIcon={ArrowLeft} onClick={handleBackClick}>
          Back
        </Button>
      </div>
      {!currentProvider ? (
        <ConfiguredProvidersView
          configuredProviders={configuredProviders}
          handleSetCurrentProvider={handleSetCurrentProvider}
          onAddInstance={handleAddInstance}
          onDuplicateProvider={handleDuplicateProvider}
        />
      ) : null}
      {currentProvider ? (
        <ProviderPreview
          currentProvider={currentProvider}
          configuredProviders={configuredProviders}
          handleSetCurrentProvider={handleSetCurrentProvider}
          onDuplicateProvider={handleDuplicateProvider}
        />
      ) : null}
      <AddProviderInstanceModal
        isOpen={instanceModalOpen}
        configuredProviders={configuredProviders}
        initialBaseProvider={initialBaseProvider}
        onOpenChange={setInstanceModalOpen}
        onCreated={handleInstanceCreated}
      />
      {information ? (
        <InformationCallout
          timeout={3000}
          mx="0"
          onClick={() => dispatch(clearInformation())}
          className={classNames(styles.popup, {
            [styles.popup_ide]: currentHost !== "web",
          })}
        >
          {information}
        </InformationCallout>
      ) : null}
      {globalError ? (
        <ErrorCallout
          mx="0"
          timeout={3000}
          onClick={() => dispatch(clearError())}
          className={classNames(styles.popup, {
            [styles.popup_ide]: currentHost !== "web",
          })}
        >
          {globalError}
        </ErrorCallout>
      ) : null}
    </div>
  );
};
