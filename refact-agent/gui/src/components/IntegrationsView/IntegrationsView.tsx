import classNames from "classnames";
import { FC, ReactNode } from "react";
import { selectConfig } from "../../features/Config/configSlice";
import { clearError, getErrorMessage } from "../../features/Errors/errorsSlice";
import {
  clearInformation,
  getInformationMessage,
} from "../../features/Errors/informationSlice";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { IntegrationWithIconResponse } from "../../services/refact";
import { ErrorCallout } from "../Callout";
import { InformationCallout } from "../Callout/Callout";
import { Spinner } from "../Spinner";
import { IntegrationsList } from "./DisplayIntegrations/IntegrationsList";
import { IntegrationsHeader } from "./Header/IntegrationsHeader";
import { IntegrationForm } from "./IntegrationForm";
import styles from "./IntegrationsView.module.css";
import { IntermediateIntegration } from "./IntermediateIntegration";
import { useIntegrations } from "./hooks/useIntegrations";

type IntegrationViewProps = {
  integrationsMap?: IntegrationWithIconResponse;
  isLoading: boolean;
  goBack?: () => void;
  handleIfInnerIntegrationWasSet: (state: boolean) => void;
  embedded?: boolean;
};

export const IntegrationsView: FC<IntegrationViewProps> = ({
  integrationsMap,
  isLoading,
  goBack,
  handleIfInnerIntegrationWasSet,
  embedded,
}) => {
  const dispatch = useAppDispatch();
  const globalError = useAppSelector(getErrorMessage);
  const information = useAppSelector(getInformationMessage);
  const config = useAppSelector(selectConfig);

  const {
    currentIntegration,
    currentNotConfiguredIntegration,
    availableIntegrationsToConfigure,
    integrationLogo,
    handleSubmit,
    handleDeleteIntegration,
    handleNotConfiguredIntegrationSubmit,
    handleMCPWizardSubmit,
    handleSetCurrentIntegrationSchema,
    handleSetCurrentIntegrationValues,
    handleFormReturn,
    goBackAndClearError,
    handleIntegrationShowUp,
    handleUpdateFormField,
    isDisabledIntegrationForm,
    isApplyingIntegrationForm,
    isDeletingIntegration,
    globalIntegrations,
    groupedProjectIntegrations,
    formValues,
  } = useIntegrations({
    integrationsMap,
    handleIfInnerIntegrationWasSet,
    goBack,
  });

  const renderHeader = (): ReactNode => {
    if (!(currentIntegration ?? currentNotConfiguredIntegration)) return null;

    return (
      <IntegrationsHeader
        handleFormReturn={handleFormReturn}
        handleInstantReturn={goBackAndClearError}
        instantBackReturn={
          currentNotConfiguredIntegration?.wasOpenedThroughChat ??
          currentIntegration?.wasOpenedThroughChat ??
          false
        }
        integrationName={
          currentIntegration?.integr_name ??
          currentNotConfiguredIntegration?.integr_name ??
          ""
        }
        icon={integrationLogo}
        embedded={embedded}
      />
    );
  };

  const renderIntegrationForm = ({
    currentHost,
  }: {
    currentHost: string;
  }): ReactNode => {
    if (!currentIntegration) return null;

    return (
      <div className={styles.content}>
        <IntegrationForm
          handleSubmit={(event) => void handleSubmit(event)}
          handleDeleteIntegration={(path) => void handleDeleteIntegration(path)}
          integrationPath={currentIntegration.integr_config_path}
          isApplying={isApplyingIntegrationForm}
          isDeletingIntegration={isDeletingIntegration}
          isDisabled={isDisabledIntegrationForm}
          formValues={formValues}
          onSchema={handleSetCurrentIntegrationSchema}
          onValues={handleSetCurrentIntegrationValues}
          handleUpdateFormField={handleUpdateFormField}
        />
        {information && (
          <InformationCallout
            timeout={isDeletingIntegration ? 1000 : 3000}
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
      </div>
    );
  };

  const renderNotConfiguredIntegration = (): ReactNode => {
    if (!currentNotConfiguredIntegration) return null;

    return (
      <div className={styles.content}>
        <IntermediateIntegration
          handleSubmit={handleNotConfiguredIntegrationSubmit}
          integration={currentNotConfiguredIntegration}
          handleMCPWizardSubmit={handleMCPWizardSubmit}
        />
      </div>
    );
  };

  if (isLoading) {
    return <Spinner spinning />;
  }

  if (!integrationsMap) {
    return (
      <ErrorCallout
        className={classNames(styles.popup, {
          [styles.popup_ide]: config.host !== "web",
        })}
        mx="0"
        onClick={goBackAndClearError}
      >
        fetching integrations.
      </ErrorCallout>
    );
  }

  const renderContent = (): ReactNode => {
    if (currentNotConfiguredIntegration) {
      return renderNotConfiguredIntegration();
    }

    if (currentIntegration) {
      return renderIntegrationForm({ currentHost: config.host });
    }

    return (
      <IntegrationsList
        globalIntegrations={globalIntegrations}
        availableIntegrationsToConfigure={availableIntegrationsToConfigure}
        groupedProjectIntegrations={groupedProjectIntegrations}
        handleIntegrationShowUp={handleIntegrationShowUp}
      />
    );
  };

  return (
    <div className={styles.root}>
      <div className={styles.content}>
        {renderHeader()}
        {renderContent()}
        {globalError && (
          <ErrorCallout
            mx="0"
            timeout={3000}
            onClick={() => dispatch(clearError())}
            className={classNames(styles.popup, {
              [styles.popup_ide]: config.host !== "web",
            })}
            preventRetry
          >
            {globalError}
          </ErrorCallout>
        )}
      </div>
    </div>
  );
};
