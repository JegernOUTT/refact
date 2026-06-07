import { Flex } from "@radix-ui/themes";
import { FC } from "react";
import { CustomBoolField, CustomInputField } from "../../components/IntegrationsView/CustomFieldsAndWidgets";
import { SmartLink } from "../../components/SmartLink";
import { ParametersTable } from "../../components/IntegrationsView/IntegrationsTable/ParametersTable";
import { Markdown } from "../../components/Markdown";
import { toPascalCase } from "../../utils/toPascalCase";
import styles from "./IntegrationFormField.module.css";
import { FieldRow } from "../../components/ui";

import {
  areToolParameters,
  IntegrationFieldValue,
  isDictionary,
  isMCPArgumentsArray,
  type Integration,
  type IntegrationField,
  type IntegrationPrimitive,
  type SmartLink as TSmartLink,
} from "../../services/refact";
import { KeyValueTable } from "../../components/IntegrationsView/IntegrationsTable/KeyValueTable";

const getDefaultValue = ({
  field,
  values,
  fieldKey,
  f_type_raw,
}: {
  field: IntegrationField<NonNullable<IntegrationPrimitive>>;
  values: Integration["integr_values"];
  fieldKey: string;
  f_type_raw: string;
}): string | number | boolean | Record<string, string> | undefined => {
  if (values && fieldKey in values) {
    return values[fieldKey]?.toString();
  }

  switch (f_type_raw) {
    case "int":
      return Number(field.f_default);
    case "bool":
      return Boolean(field.f_default);
    case "tool_parameters":
    case "output_filter":
      return JSON.stringify(field.f_default);
    case "string_to_string_map":
      return field.f_default as Record<string, string>;
    case "string":
      return field.f_default?.toString();
    default:
      return field.f_default?.toString();
  }
};

type IntegrationFormFieldProps = {
  field: IntegrationField<NonNullable<IntegrationPrimitive>>;
  values: Integration["integr_values"];
  fieldKey: string;
  integrationName: string;
  integrationPath: string;
  integrationProject: string;
  isFieldVisible?: boolean;
  onChange: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
};

type CommonFieldProps = {
  id: string;
  name: string;
  value?: string | number | boolean | Record<string, string>;
  placeholder?: string;
};

const FieldContent: FC<{
  f_type: string;
  commonProps: CommonFieldProps;
  values: Integration["integr_values"];
  fieldKey: string;
  onChange: (fieldKey: string, fieldValue: IntegrationFieldValue) => void;
}> = ({ f_type, commonProps, values, fieldKey, onChange }) => {
  switch (f_type) {
    case "bool": {
      return (
        <CustomBoolField
          {...commonProps}
          value={Boolean(commonProps.value ?? values?.[fieldKey] ?? false)}
          onChange={(value) => onChange(fieldKey, value)}
        />
      );
    }
    case "tool_parameters": {
      const valuesForTable = values?.[fieldKey] ?? [];
      if (areToolParameters(valuesForTable)) {
        return (
          <ParametersTable initialData={valuesForTable} onToolParameters={(data) => onChange(fieldKey, data)} />
        );
      }
      break;
    }
    case "output_filter": {
      return (
        <div className={styles.scrollX}>
          <Markdown>
            {"```json\n" + JSON.stringify(values ? values[fieldKey] : {}, null, 2) + "\n```"}
          </Markdown>
        </div>
      );
    }
    case "string_array": {
      const valuesForTable = values?.[fieldKey];
      const tableData = isMCPArgumentsArray(valuesForTable) ? valuesForTable : [];

      return (
        <KeyValueTable
          initialData={tableData as unknown as Record<string, string>}
          onChange={(data) => onChange(fieldKey, data)}
        />
      );
    }
    case "string_to_string_map": {
      const valuesForTable = values?.[fieldKey] ?? commonProps.value;
      const tableData = isDictionary(valuesForTable) ? valuesForTable : {};

      const columnsMapToArray: Record<string, string[]> = {
        env: ["Environment Variable", "Value"],
        headers: ["Header Name", "Value"],
      };
      const emptyMessageMap: Record<string, string> = {
        env: "No environment variables specified yet",
        headers: "No headers specified yet",
      };

      return (
        <KeyValueTable
          initialData={tableData}
          onChange={(data) => onChange(fieldKey, data)}
          columnNames={columnsMapToArray[fieldKey]}
          emptyMessage={emptyMessageMap[fieldKey]}
        />
      );
    }
    case "string_short": {
      return (
        <CustomInputField
          {...commonProps}
          type="text"
          size="short"
          value={commonProps.value?.toString()}
          onChange={(value) => onChange(fieldKey, value)}
        />
      );
    }
    case "string_long": {
      return (
        <CustomInputField
          {...commonProps}
          type="text"
          size="long"
          value={commonProps.value?.toString()}
          onChange={(value) => onChange(fieldKey, value)}
        />
      );
    }
    default: {
      return (
        <CustomInputField
          {...commonProps}
          type="text"
          size="short"
          value={commonProps.value?.toString()}
          onChange={(value) => onChange(fieldKey, value)}
        />
      );
    }
  }
};

const SmartLinks: FC<{
  smartlinks: TSmartLink[] | undefined;
  integrationName: string;
  integrationPath: string;
  integrationProject: string;
}> = ({ smartlinks, integrationName, integrationPath, integrationProject }) => {
  if (!smartlinks || smartlinks.every((link) => link.sl_goto?.startsWith("EDITOR"))) {
    return null;
  }

  return (
    <Flex align="center" className={styles.smartLinks}>
      {smartlinks.map((smartlink, index) => (
        <SmartLink
          isSmall
          key={`smartlink-${index}`}
          smartlink={smartlink}
          integrationName={integrationName}
          integrationPath={integrationPath}
          integrationProject={integrationProject}
        />
      ))}
    </Flex>
  );
};

export const IntegrationFormField: FC<IntegrationFormFieldProps> = ({
  field,
  values,
  fieldKey,
  integrationName,
  integrationPath,
  integrationProject,
  isFieldVisible = true,
  onChange,
}) => {
  const f_type_raw = field.f_type.toString();

  const commonProps = {
    id: fieldKey,
    name: fieldKey,
    value:
      getDefaultValue({
        f_type_raw,
        field,
        fieldKey,
        values,
      }) ?? "",
    placeholder: field.f_placeholder?.toString(),
  };

  return (
    <FieldRow
      key={fieldKey}
      className={styles.field}
      data-hidden={!isFieldVisible ? true : undefined}
      helper={field.f_desc ? <Markdown>{field.f_desc}</Markdown> : undefined}
      htmlFor={fieldKey}
      label={field.f_label ?? toPascalCase(fieldKey)}
    >
      <div className={styles.controlStack}>
        <FieldContent
          f_type={f_type_raw}
          commonProps={commonProps}
          values={values}
          fieldKey={fieldKey}
          onChange={onChange}
        />

        <SmartLinks
          smartlinks={field.smartlinks}
          integrationName={integrationName}
          integrationPath={integrationPath}
          integrationProject={integrationProject}
        />
      </div>
    </FieldRow>
  );
};
