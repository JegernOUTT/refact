import React from "react";
import { Badge, Button, Flex, Separator, Text } from "@radix-ui/themes";

import { SchemaField } from "./SchemaField";
import { ProviderOAuth } from "./ProviderOAuth";
import { Spinner } from "../../../components/Spinner";

import { useProviderForm } from "./useProviderForm";
import type {
  ProviderListItem,
  ProviderStatus,
} from "../../../services/refact";

import styles from "./ProviderForm.module.css";
import { ProviderModelsList } from "./ProviderModelsList/ProviderModelsList";
import { useGetOpenRouterHealthQuery } from "../../../services/refact";

const SETTINGS_HIDDEN_PROVIDERS = ["refact", "refact_self_hosted"];

export type ProviderFormProps = {
  currentProvider: ProviderListItem;
};

export type { ProviderListItem };

const StatusBadge: React.FC<{ status: ProviderStatus }> = ({ status }) => {
  switch (status) {
    case "active":
      return (
        <Badge color="green" size="1">
          Active
        </Badge>
      );
    case "configured":
      return (
        <Badge color="orange" size="1">
          Configured
        </Badge>
      );
    case "not_configured":
      return (
        <Badge color="gray" size="1">
          Not configured
        </Badge>
      );
    default:
      return null;
  }
};

export const ProviderForm: React.FC<ProviderFormProps> = ({
  currentProvider,
}) => {
  const { data: openRouterHealth } = useGetOpenRouterHealthQuery(undefined, {
    skip: currentProvider.name !== "openrouter",
  });
  const {
    areShowingExtraFields,
    formValues,
    parsedSchema,
    importantFields,
    extraFields,
    isProviderLoadedSuccessfully,
    setAreShowingExtraFields,
    handleFieldSave,
    detailedProvider,
  } = useProviderForm({ providerName: currentProvider.name });

  if (!isProviderLoadedSuccessfully || !formValues || !parsedSchema) {
    return <Spinner spinning />;
  }

  const hideSettings = SETTINGS_HIDDEN_PROVIDERS.includes(currentProvider.name);
  const hasOAuth = parsedSchema.oauth?.supported === true;
  const status: ProviderStatus =
    detailedProvider?.status ?? currentProvider.status;
  const hasCredentials =
    detailedProvider?.has_credentials ?? currentProvider.has_credentials;
  const isReadonly = formValues.readonly;

  return (
    <Flex direction="column" width="100%" height="100%" mt="2" gap="3">
      <Flex align="center" gap="2">
        <StatusBadge status={status} />
        {currentProvider.name === "openrouter" && openRouterHealth && (
          <Badge color={openRouterHealth.ok ? "green" : "red"} size="1">
            {openRouterHealth.ok ? "Key OK" : "Key Error"}
          </Badge>
        )}
        {parsedSchema.description && (
          <Text size="1" color="gray" style={{ flex: 1 }}>
            {parsedSchema.description.trim().split("\n")[0]}
          </Text>
        )}
      </Flex>

      {!hideSettings && (
        <Flex direction="column" width="100%" gap="3">
          {hasOAuth && (
            <>
              <ProviderOAuth
                providerName={currentProvider.name}
                oauthConnected={Boolean(
                  "oauth_connected" in formValues && formValues.oauth_connected,
                )}
                authStatus={
                  "auth_status" in formValues
                    ? String(formValues.auth_status)
                    : ""
                }
              />
              {importantFields.length > 0 && <Separator size="4" />}
            </>
          )}

          <Flex direction="column" gap="3">
            {importantFields.map((field) => (
              <SchemaField
                key={field.key}
                field={field}
                value={formValues[field.key]}
                disabled={isReadonly}
                onSave={handleFieldSave}
              />
            ))}
          </Flex>

          {extraFields.length > 0 && (
            <>
              <Flex align="center" justify="center">
                <Button
                  className={styles.extraButton}
                  variant="ghost"
                  color="gray"
                  size="1"
                  onClick={() => setAreShowingExtraFields((prev) => !prev)}
                >
                  {areShowingExtraFields ? "Hide" : "Show"} advanced fields
                </Button>
              </Flex>

              {areShowingExtraFields && (
                <Flex direction="column" gap="3">
                  {extraFields.map((field) => (
                    <SchemaField
                      key={field.key}
                      field={field}
                      value={formValues[field.key]}
                      disabled={isReadonly}
                      onSave={handleFieldSave}
                    />
                  ))}
                </Flex>
              )}
            </>
          )}
        </Flex>
      )}

      {(hasCredentials || hideSettings) && (
        <ProviderModelsList provider={currentProvider} />
      )}
    </Flex>
  );
};
