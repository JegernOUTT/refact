import { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { useCallback, useMemo, useState } from "react";
import {
  areAllFieldsBoolean,
  integrationsApi,
  IntegrationWithIconRecord,
  isDetailMessage,
  NotConfiguredIntegrationWithIconRecord,
} from "../../../services/refact";
import { setError } from "../../../features/Errors/errorsSlice";
import { useAppDispatch } from "../../../hooks";

const getAvailabilityErrorMessage = (error: unknown, fallback: string) => {
  if (error && typeof error === "object" && "data" in error) {
    const data = (error as FetchBaseQueryError).data;
    if (isDetailMessage(data)) {
      return data.detail;
    }
  }

  if (error && typeof error === "object" && "error" in error) {
    const message = (error as { error?: unknown }).error;
    if (typeof message === "string") {
      return message;
    }
  }

  if (error instanceof Error) {
    return error.message;
  }

  return fallback;
};

export const useUpdateIntegration = ({
  integration,
}: {
  integration:
    | IntegrationWithIconRecord
    | NotConfiguredIntegrationWithIconRecord;
}) => {
  const dispatch = useAppDispatch();

  const [getIntegrationData] =
    integrationsApi.useLazyGetIntegrationByPathQuery();
  const [saveIntegrationData] = integrationsApi.useSaveIntegrationMutation();
  const [updatedAvailability, setUpdatedAvailability] = useState<
    Record<string, boolean>
  >({
    on_your_laptop: integration.on_your_laptop,
    when_isolated: integration.when_isolated,
  });

  const [isUpdatingAvailability, setIsUpdatingAvailability] = useState(false);

  const updateIntegrationAvailability = useCallback(async () => {
    if (Array.isArray(integration.integr_config_path)) {
      return;
    }

    setIsUpdatingAvailability(true);
    try {
      const response = await getIntegrationData(integration.integr_config_path);

      if (response.error) {
        dispatch(
          setError(
            getAvailabilityErrorMessage(
              response.error,
              `Failed to fetch ${integration.integr_name} configuration before updating availability`,
            ),
          ),
        );
        return;
      }

      const integrationData = response.data;

      if (!integrationData?.integr_values) {
        dispatch(
          setError(
            `${integration.integr_name} configuration has no saved values to update availability`,
          ),
        );
        return;
      }

      const { available } = integrationData.integr_values;
      const newAvailability = areAllFieldsBoolean(available)
        ? {
            on_your_laptop: !available.on_your_laptop,
            when_isolated: available.when_isolated,
          }
        : {
            on_your_laptop: integration.on_your_laptop,
            when_isolated: integration.when_isolated,
          };

      const saveResponse = await saveIntegrationData({
        filePath: integration.integr_config_path,
        values: {
          ...integrationData.integr_values,
          available: newAvailability,
        },
      });
      if (saveResponse.error) {
        dispatch(
          setError(
            getAvailabilityErrorMessage(
              saveResponse.error,
              `Error occurred on updating ${integration.integr_name} configuration. Check if your integration configuration is correct`,
            ),
          ),
        );
        return;
      }

      setUpdatedAvailability(newAvailability);
    } finally {
      setIsUpdatingAvailability(false);
    }
  }, [dispatch, getIntegrationData, saveIntegrationData, integration]);

  const integrationAvailability = useMemo(() => {
    return updatedAvailability;
  }, [updatedAvailability]);

  return {
    updateIntegrationAvailability,
    integrationAvailability,
    isUpdatingAvailability,
  };
};
