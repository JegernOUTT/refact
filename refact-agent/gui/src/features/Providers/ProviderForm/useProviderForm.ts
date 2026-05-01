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

export type ProviderFormValues = {
  enabled: boolean;
  readonly: boolean;
  base_provider: string;
  display_name: string;
  [key: string]: unknown;
};

type ParsedSchema = {
  fields: SchemaFieldDef[];
  oauth?: {
    supported: boolean;
    methods?: { id: string; label: string; description?: string }[];
  };
  description?: string;
};

const jsYamlPromise = import("js-yaml");

async function parseSchema(yamlStr: string): Promise<ParsedSchema> {
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
        f_default: def.f_default ? String(def.f_default) : undefined,
        f_extra: Boolean(def.f_extra),
        f_secret: Boolean(def.f_secret),
        smartlinks: Array.isArray(def.smartlinks)
          ? def.smartlinks.map((sl: Record<string, unknown>) => ({
              sl_label: String(sl.sl_label ?? ""),
              sl_goto: String(sl.sl_goto ?? ""),
            }))
          : undefined,
      });
    }
  }

  const oauth = parsed.oauth as ParsedSchema["oauth"] | undefined;
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
    };
  }, [providerDetail]);

  const { importantFields, extraFields } = useMemo(() => {
    if (!parsedSchema) return { importantFields: [], extraFields: [] };
    const important: SchemaFieldDef[] = [];
    const extra: SchemaFieldDef[] = [];
    for (const field of parsedSchema.fields) {
      if (field.f_extra) {
        extra.push(field);
      } else {
        important.push(field);
      }
    }
    return { importantFields: important, extraFields: extra };
  }, [parsedSchema]);

  const [updateProvider] = providersApi.useUpdateProviderMutation();

  const handleFieldSave = useCallback(
    async (key: string, value: unknown) => {
      if (!providerDetail) return;
      const response = await updateProvider({
        providerName,
        settings: {
          ...providerIdentitySettings(providerDetail),
          [key]: value,
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

  return {
    formValues,
    parsedSchema,
    importantFields,
    extraFields,
    areShowingExtraFields,
    setAreShowingExtraFields,
    handleFieldSave,
    configuredProviders,
    detailedProvider,
    isProviderLoadedSuccessfully,
  };
}
