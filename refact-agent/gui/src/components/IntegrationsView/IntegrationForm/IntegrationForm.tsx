import classNames from "classnames";
import { FC, FormEvent, useEffect } from "react";
import { useGetIntegrationDataByPathQuery } from "../../../hooks/useGetIntegrationDataByPathQuery";
import { Confirmation } from "../Confirmation";
import { useFormFields } from "../hooks/useFormFields";

import {
  areToolConfirmation,
  IntegrationFieldValue,
  type Integration,
} from "../../../services/refact";
import { Button, Flex, Spinner, Surface, Text } from "../../ui";
import { ErrorState } from "./ErrorState";
import { FormAvailabilityAndDelete } from "./FormAvailabilityAndDelete";
import { FormFields } from "./FormFields";
import { FormSmartlinks } from "./FormSmartlinks";
import styles from "./IntegrationForm.module.css";
import { MCPServerView } from "../MCPServerView";

type IntegrationFormProps = {
  integrationPath: string;
  isApplying: boolean;
  isDisabled: boolean;
  isDeletingIntegration: boolean;
  handleSubmit: (event: FormEvent<HTMLFormElement>) => void;
  handleDeleteIntegration: (path: string) => void;
  onSchema: (schema: Integration["integr_schema"]) => void;
  onValues: (values: Integration["integr_values"]) => void;
  handleUpdateFormField: (
    fieldKey: string,
    fieldValue: IntegrationFieldValue,
  ) => void;
  formValues: Integration["integr_values"];
};

export const IntegrationForm: FC<IntegrationFormProps> = ({
  integrationPath,
  isApplying,
  isDisabled,
  isDeletingIntegration,
  handleSubmit,
  handleDeleteIntegration,
  onSchema,
  onValues,
  handleUpdateFormField,
  formValues,
}) => {
  const { integration } = useGetIntegrationDataByPathQuery(integrationPath);

  const {
    importantFields,
    extraFields,
    areExtraFieldsRevealed,
    toggleExtraFields,
  } = useFormFields(integration.data?.integr_schema.fields, integrationPath);

  const schema = integration.data?.integr_schema;
  const values = integration.data?.integr_values;

  useEffect(() => {
    if (schema) {
      onSchema(schema);
    }
  }, [schema, onSchema]);

  useEffect(() => {
    if (values) {
      onValues(values);
    }
  }, [values, onValues]);

  if (integration.isLoading) {
    return <Spinner />;
  }

  if (!integration.data) {
    return <Text>No integration found</Text>;
  }

  if (integration.data.error_log.length > 0) {
    return (
      <ErrorState
        integration={integration.data}
        onDelete={handleDeleteIntegration}
        isApplying={isApplying}
        isDeletingIntegration={isDeletingIntegration}
      />
    );
  }

  const hasExtraFields = Object.keys(extraFields).length > 0;
  const isMcpIntegration = integration.data.integr_name.includes("mcp");

  return (
    <Flex
      className={classNames(styles.root, "rf-enter")}
      direction="column"
      gap="3"
    >
      {integration.data.integr_schema.description && (
        <Text as="p" className={styles.description} size="2" color="gray">
          {integration.data.integr_schema.description}
        </Text>
      )}

      <Surface
        as="form"
        animated
        className={styles.formSurface}
        id={`form-${integration.data.integr_name}`}
        onSubmit={handleSubmit}
        radius="card"
        variant="glass"
      >
        <Flex className="rf-stagger" direction="column" gap="3">
          <div className={styles.formGrid}>
            <FormAvailabilityAndDelete
              integration={integration.data}
              isApplying={isApplying}
              isDeletingIntegration={isDeletingIntegration}
              formValues={formValues}
              onDelete={handleDeleteIntegration}
              onChange={handleUpdateFormField}
            />
            <FormSmartlinks
              integration={integration.data}
              smartlinks={integration.data.integr_schema.smartlinks}
            />
            <FormFields
              integration={integration.data}
              importantFields={importantFields}
              extraFields={extraFields}
              areExtraFieldsRevealed={areExtraFieldsRevealed}
              values={formValues}
              onChange={handleUpdateFormField}
            />
          </div>

          {hasExtraFields && (
            <Button
              type="button"
              variant="ghost"
              onClick={toggleExtraFields}
              className={styles.advancedButton}
            >
              {areExtraFieldsRevealed
                ? "Hide advanced configuration"
                : "Show advanced configuration"}
            </Button>
          )}

          {!integration.data.integr_schema.confirmation.not_applicable && (
            <div className={styles.confirmationWrap}>
              <Confirmation
                confirmationByUser={
                  areToolConfirmation(formValues?.confirmation)
                    ? formValues.confirmation
                    : null
                }
                confirmationFromValues={
                  areToolConfirmation(
                    integration.data.integr_values?.confirmation,
                  )
                    ? integration.data.integr_values.confirmation
                    : null
                }
                defaultConfirmationObject={
                  integration.data.integr_schema.confirmation
                }
                onChange={handleUpdateFormField}
              />
            </div>
          )}

          <div
            className={classNames(styles.actionBar, {
              [styles.stickyActionBar]: isMcpIntegration,
              [styles.fixedActionBar]: !isMcpIntegration,
            })}
          >
            <Button
              variant="primary"
              type="submit"
              size="md"
              title={isDisabled ? "Cannot apply, no changes made" : "Apply"}
              className={classNames(styles.button, styles.applyButton, {
                [styles.disabledButton]: isApplying || isDisabled,
              })}
              disabled={isDisabled || isApplying}
            >
              {isApplying ? "Applying..." : "Apply"}
            </Button>
          </div>
        </Flex>
      </Surface>

      {isMcpIntegration && integration.data.integr_values !== null && (
        <>
          <div className={styles.divider} role="separator" />
          <MCPServerView
            configPath={integration.data.integr_config_path}
            integrName={integration.data.integr_name}
          />
        </>
      )}
    </Flex>
  );
};
