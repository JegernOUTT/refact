import React, { useCallback, useState } from "react";
import { Button, Flex } from "@radix-ui/themes";
import { ArrowLeftIcon } from "@radix-ui/react-icons";

import { ConfiguredProvidersView } from "./ConfiguredProvidersView";

import type { ProviderListItem } from "../../../services/refact";
import { ProviderPreview } from "../ProviderPreview";
import {
  ErrorCallout,
  InformationCallout,
} from "../../../components/Callout/Callout";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { clearError, getErrorMessage } from "../../Errors/errorsSlice";
import {
  clearInformation,
  getInformationMessage,
} from "../../Errors/informationSlice";

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

  const [currentProvider, setCurrentProvider] =
    useState<ProviderListItem | null>(null);
  const handleSetCurrentProvider = useCallback(
    (provider: ProviderListItem | null) => {
      setCurrentProvider(provider);
    },
    [],
  );

  const handleBackClick = useCallback(() => {
    if (currentProvider) {
      setCurrentProvider(null);
    } else {
      backFromProviders();
    }
  }, [currentProvider, backFromProviders]);

  return (
    <Flex px="1" direction="column" height="100%" width="100%">
      {currentHost === "vscode" ? (
        <Flex gap="2" pb="3">
          <Button variant="surface" onClick={handleBackClick}>
            <ArrowLeftIcon width="16" height="16" />
            Back
          </Button>
        </Flex>
      ) : (
        <Button mr="auto" variant="outline" onClick={handleBackClick} mb="4">
          Back
        </Button>
      )}
      {!currentProvider && (
        <ConfiguredProvidersView
          configuredProviders={configuredProviders}
          handleSetCurrentProvider={handleSetCurrentProvider}
        />
      )}
      {currentProvider && (
        <ProviderPreview
          currentProvider={currentProvider}
          configuredProviders={configuredProviders}
          handleSetCurrentProvider={handleSetCurrentProvider}
        />
      )}
      {information && (
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
      )}
      {globalError && (
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
      )}
    </Flex>
  );
};
