import React, { useCallback, useEffect, useState } from "react";
import { ArrowLeft } from "lucide-react";
import classNames from "classnames";
import { ScrollArea } from "../../components/ScrollArea";
import { PageWrapper } from "../../components/PageWrapper";
import type { Config } from "../Config/configSlice";
import { useGetIntegrationsQuery } from "../../hooks/useGetIntegrationsDataQuery";
import { IntegrationsView } from "../../components/IntegrationsView";
import { Button } from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import { integrationsApi } from "../../services/refact/integrations";
import styles from "./Integrations.module.css";

export type IntegrationsProps = {
  onCloseIntegrations?: () => void;
  backFromIntegrations: () => void;
  handlePaddingShift: (state: boolean) => void;
  host: Config["host"];
  tabbed: Config["tabbed"];
  embedded?: boolean;
};

export const Integrations: React.FC<IntegrationsProps> = ({
  onCloseIntegrations,
  backFromIntegrations,
  handlePaddingShift,
  host,
  tabbed,
  embedded,
}) => {
  const dispatch = useAppDispatch();

  useEffect(() => {
    return () => {
      dispatch(integrationsApi.util.resetApiState());
    };
  }, [dispatch]);

  const { integrations } = useGetIntegrationsQuery();
  const [isInnerIntegrationSelected, setIsInnerIntegrationSelected] =
    useState<boolean>(false);

  const handleIfInnerIntegrationWasSet = useCallback(
    (state: boolean) => {
      setIsInnerIntegrationSelected(state);
      handlePaddingShift(state);
    },
    [handlePaddingShift],
  );

  const content = (
    <div
      className={classNames(styles.page, {
        [styles.pageInnerSelected]: isInnerIntegrationSelected && !embedded,
      })}
    >
      {!embedded && !isInnerIntegrationSelected && (
        <>
          {host === "vscode" && !tabbed ? (
            <div className={styles.backRow}>
              <Button
                variant="soft"
                onClick={backFromIntegrations}
                leftIcon={ArrowLeft}
              >
                Back
              </Button>
            </div>
          ) : (
            <Button
              className={styles.webBackButton}
              variant="ghost"
              onClick={onCloseIntegrations}
            >
              Back
            </Button>
          )}
        </>
      )}
      <ScrollArea scrollbars="vertical" fullHeight>
        <div className={styles.scrollContent}>
          <IntegrationsView
            handleIfInnerIntegrationWasSet={handleIfInnerIntegrationWasSet}
            integrationsMap={integrations.data}
            isLoading={integrations.isLoading}
            goBack={backFromIntegrations}
            embedded={embedded}
          />
        </div>
      </ScrollArea>
    </div>
  );

  if (embedded) return content;
  return (
    <PageWrapper host={host} noPadding>
      {content}
    </PageWrapper>
  );
};
