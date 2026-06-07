import * as RadioGroupPrimitive from "@radix-ui/react-radio-group";
import { type FormEvent, type FC, useState, useMemo } from "react";
import { NotConfiguredIntegrationWithIconRecord } from "../../../services/refact";
import { Button } from "../../ui";
import { CustomInputField } from "../CustomFieldsAndWidgets";
import { Link } from "../../Link";
import { useGetIntegrationDataByPathQuery } from "../../../hooks/useGetIntegrationDataByPathQuery";
import { validateSnakeCase } from "../../../utils/validateSnakeCase";
import { createProjectLabelsWithConflictMarkers } from "../../../utils/createProjectLabelsWithConflictMarkers";
import { IntegrationPathField } from "./IntegrationPathField";
import { MCPSetupWizard } from "../MCPSetupWizard";
import styles from "./IntermediateIntegration.module.css";

type IntegrationCmdlineProps = {
  integration: NotConfiguredIntegrationWithIconRecord;
  handleSubmit: (event: FormEvent<HTMLFormElement>) => void;
  handleMCPWizardSubmit?: (configPath: string, integrName: string) => void;
};

export const IntermediateIntegration: FC<IntegrationCmdlineProps> = ({
  integration,
  handleSubmit,
  handleMCPWizardSubmit,
}) => {
  const isMCP =
    integration.integr_name === "mcp_TEMPLATE" ||
    (integration.integr_name.startsWith("mcp") &&
      integration.integr_name.endsWith("TEMPLATE"));

  const [integrationType, integrationTemplate] =
    integration.integr_name.split("_");
  const [commandName, setCommandName] = useState(
    integrationType === "cmdline" || integrationType === "service"
      ? integration.commandName
      : "",
  );
  const [errorMessage, setErrorMessage] = useState("");

  const { integration: relatedIntegration } = useGetIntegrationDataByPathQuery(
    integration.integr_config_path[0],
  );

  const projectLabels = useMemo(() => {
    const validProjectPaths = integration.project_path.filter(
      (path) => path !== "",
    );
    return createProjectLabelsWithConflictMarkers(validProjectPaths);
  }, [integration.project_path]);

  const handleCommandNameChange = (value: string) => {
    setCommandName(value);
    if (!validateSnakeCase(value)) {
      setErrorMessage("The command name must be in snake case!");
    } else {
      setErrorMessage("");
    }
  };

  if (isMCP && handleMCPWizardSubmit) {
    return (
      <MCPSetupWizard
        integration={integration}
        onSubmit={handleMCPWizardSubmit}
      />
    );
  }

  return (
    <div className={styles.root}>
      {relatedIntegration.data?.integr_schema.description && (
        <p className={styles.text}>
          {relatedIntegration.data.integr_schema.description}
        </p>
      )}
      <p className={styles.text}>
        Where do you want to configure this integration? Any project that has
        version control can have its own integrations configured.
      </p>
      <form onSubmit={handleSubmit} id={`form-${integration.integr_name}`}>
        <div className={styles.formStack}>
          <RadioGroupPrimitive.Root
            className={styles.pathGroup}
            name="integr_config_path"
            defaultValue={integration.integr_config_path[0]}
          >
            {integration.integr_config_path.map((configPath, index) => {
              const shouldPathBeFormatted =
                integration.project_path[index] !== "";

              return (
                <label className={styles.pathOption} key={configPath}>
                  <IntegrationPathField
                    configPath={configPath}
                    projectPath={integration.project_path[index]}
                    projectLabels={projectLabels}
                    shouldBeFormatted={shouldPathBeFormatted}
                  />
                </label>
              );
            })}
          </RadioGroupPrimitive.Root>
          <div className={styles.fieldStack}>
            {integrationTemplate && (
              <div className={styles.fieldStack}>
                <p className={styles.text}>
                  Name for your new command, make sure that it&apos;s written in{" "}
                  <Link
                    href="https://en.wikipedia.org/wiki/Snake_case"
                    target="_blank"
                  >
                    snake case
                  </Link>
                </p>
                <div className={styles.fieldStack}>
                  <CustomInputField
                    name="command_name"
                    placeholder="runserver_py"
                    value={commandName}
                    onChange={handleCommandNameChange}
                    color={errorMessage ? "red" : undefined}
                    wasInteracted
                  />
                  {errorMessage && <p className={styles.error}>{errorMessage}</p>}
                </div>
              </div>
            )}
            <Button
              type="submit"
              variant="primary"
              disabled={
                integrationTemplate ? !!errorMessage || !commandName : false
              }
              title={
                !!errorMessage || !commandName
                  ? "Please, fill out all required fields first"
                  : "Continue setting up integration"
              }
            >
              Continue with setup
            </Button>
          </div>
        </div>
      </form>
    </div>
  );
};
