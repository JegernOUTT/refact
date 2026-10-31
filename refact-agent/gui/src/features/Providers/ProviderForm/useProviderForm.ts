import { useCallback, useEffect, useMemo, useState } from "react";
import type { ProviderDetailResponse } from "../../../services/refact";
import { providerIdentitySettings } from "../../../services/refact";
import {
  useGetConfiguredProvidersQuery,
  useGetProviderQuery,
} from "../../../hooks/useProvidersQuery";
import {
  providersApi,
  useGetProviderSchemaQuery,
} from "../../../services/refact";
import { useAppDispatch } from "../../../hooks";
import type { SchemaFieldDef } from "./SchemaField";
import {
  effectiveClaudeBackend,
  isFieldVisible,
  parseConditionList,
  parseConditionMap,
  parseOAuthSchema,
  parseSchemaOptions,
  visibleFieldOptions,
} from "./schemaConditions";
import type { ParsedOAuthSchema } from "./schemaConditions";

export type ProviderFormValues = {
  enabled: boolean;
  readonly: boolean;
  base_provider: string;
  display_name: string;
  [key: string]: unknown;
};

type ParsedSchema = {
  fields: SchemaFieldDef[];
  oauth?: ParsedOAuthSchema;
  description?: string;
};

const jsYamlPromise = import("js-yaml");

export async function parseSchema(yamlStr: string): Promise<ParsedSchema> {
  const jsYaml = await jsYamlPromise;
  const parsed = jsYaml.load(yamlStr) as Record<string, unknown> | null;
  if (!parsed || typeof parsed !== "object") {
    return { fields: [] };
  }

  const fields: SchemaFieldDef[] = [];
  const rawFields = parsed.fields as
    | Record<string, Record<string, unknown>>
    | undefined;
  if (rawFields && typeof rawFields === "object") {
    for (const [key, def] of Object.entries(rawFields)) {
      fields.push({
        key,
        f_type: String(def.f_type ?? "string"),
        f_desc: def.f_desc ? String(def.f_desc) : undefined,
        f_label: def.f_label ? String(def.f_label) : undefined,
        f_placeholder: def.f_placeholder
          ? String(def.f_placeholder)
          : undefined,
        f_default:
          def.f_default !== undefined && def.f_default !== null
            ? String(def.f_default)
            : undefined,
        f_options: parseSchemaOptions(def.f_options),
        f_visible_when: parseConditionMap(def.f_visible_when),
        f_extra: Boolean(def.f_extra),
        f_visible_when_any: parseConditionList(def.f_visible_when_any),
        f_secret: Boolean(def.f_secret),
        f_object: Boolean(def.f_object),
        f_confirmation: Boolean(def.f_confirmation),
        smartlinks: Array.isArray(def.smartlinks)
          ? def.smartlinks.map((sl: Record<string, unknown>) => ({
              sl_label: String(sl.sl_label ?? ""),
              sl_goto: String(sl.sl_goto ?? ""),
            }))
          : undefined,
      });
    }
  }

  const oauth = parseOAuthSchema(parsed.oauth);
  const description = parsed.description
    ? String(parsed.description)
    : undefined;

  return { fields, oauth, description };
}

export function useProviderForm({ providerName }: { providerName: string }) {
  const dispatch = useAppDispatch();
  const { data: providerDetail, isSuccess: isProviderLoadedSuccessfully } =
    useGetProviderQuery({ providerName });
  const { data: schemaData } = useGetProviderSchemaQuery({ providerName });
  const { data: configuredProviders } = useGetConfiguredProvidersQuery();

  const [parsedSchema, setParsedSchema] = useState<ParsedSchema | null>(null);
  const [areShowingExtraFields, setAreShowingExtraFields] = useState(false);

  useEffect(() => {
    if (schemaData?.schema) {
      void parseSchema(schemaData.schema).then(setParsedSchema);
    }
  }, [schemaData?.schema]);

  const formValues: ProviderFormValues | null = useMemo(() => {
    if (!providerDetail) return null;
    return {
      enabled: providerDetail.enabled,
      readonly: providerDetail.readonly,
      base_provider: providerDetail.base_provider,
      display_name: providerDetail.display_name,
      ...providerDetail.settings,
      ...(effectiveClaudeBackend(
        providerDetail.base_provider,
        providerDetail.settings,
      )
        ? {
            backend: effectiveClaudeBackend(
              providerDetail.base_provider,
              providerDetail.settings,
            ),
          }
        : {}),
    };
  }, [providerDetail]);

  const { importantFields, extraFields } = useMemo(() => {
    if (!parsedSchema) return { importantFields: [], extraFields: [] };
    const important: SchemaFieldDef[] = [];
    const extra: SchemaFieldDef[] = [];
    const values = formValues ?? {};
    const baseProvider = formValues?.base_provider ?? "";
    for (const schemaField of parsedSchema.fields) {
      if (!isFieldVisible(schemaField, parsedSchema.fields, values)) continue;
      const field = schemaField.f_options
        ? {
            ...schemaField,
            f_options: visibleFieldOptions(
              schemaField,
              parsedSchema.fields,
              values,
              baseProvider,
            ),
          }
        : schemaField;
      if (field.f_extra) {
        extra.push(field);
      } else {
        important.push(field);
      }
    }
    return { importantFields: important, extraFields: extra };
  }, [parsedSchema, formValues]);

  const [updateProvider] = providersApi.useUpdateProviderMutation();

  const handleSettingsSave = useCallback(
    async (settings: Record<string, unknown>) => {
      if (!providerDetail) throw new Error("Provider not loaded");
      const response = await updateProvider({
        providerName,
        settings: {
          ...providerIdentitySettings(providerDetail),
          ...settings,
        },
      });
      if (response.error) {
        throw new Error("Failed to save");
      }
      dispatch(
        providersApi.util.invalidateTags([
          { type: "PROVIDER", id: providerName },
          { type: "PROVIDERS", id: "LIST" },
          { type: "AVAILABLE_MODELS", id: providerName },
        ]),
      );
    },
    [providerDetail, providerName, updateProvider, dispatch],
  );

  const detailedProvider: ProviderDetailResponse | undefined = providerDetail;

  const handleFieldSave = useCallback(
    (key: string, value: unknown) => handleSettingsSave({ [key]: value }),
    [handleSettingsSave],
  );

  return {
    formValues,
    parsedSchema,
    importantFields,
    extraFields,
    areShowingExtraFields,
    setAreShowingExtraFields,
    handleFieldSave,
    handleSettingsSave,
    configuredProviders,
    detailedProvider,
    isProviderLoadedSuccessfully,
  };
}
