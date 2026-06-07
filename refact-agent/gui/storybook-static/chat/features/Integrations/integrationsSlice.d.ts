import { Selector } from 'reselect';
import { IntegrationFieldValue } from '../../services/refact';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, type PayloadAction } from '@reduxjs/toolkit';
import { IntegrationPrimitive, Integration } from "../../services/refact/integrations";
type FormKeyValueMap = Integration["integr_values"];
export type IntegrationCachedFormData = Record<string, FormKeyValueMap>;
export declare const integrationsSlice: Slice<{
    cachedForms: IntegrationCachedFormData;
}, {
    addToCacheOnMiss: (state: WritableDraft<{
        cachedForms: IntegrationCachedFormData;
    }>, action: PayloadAction<Integration>) => void;
    removeFromCache: (state: WritableDraft<{
        cachedForms: IntegrationCachedFormData;
    }>, action: PayloadAction<string>) => void;
    clearCache: (state: WritableDraft<{
        cachedForms: IntegrationCachedFormData;
    }>) => void;
}, "integrations", "integrations", {
    maybeSelectIntegrationFromCache: (state: {
        cachedForms: IntegrationCachedFormData;
    }, integration: Integration) => Record<string, IntegrationFieldValue> | null;
    checkValuesForChanges: (_state: {
        cachedForms: IntegrationCachedFormData;
    }, _integration: Integration) => (_accessors: string | string[], _value: IntegrationPrimitive) => false;
}>;
export declare const addToCacheOnMiss: ActionCreatorWithPayload<Integration, "integrations/addToCacheOnMiss">, removeFromCache: ActionCreatorWithPayload<string, "integrations/removeFromCache">;
export declare const maybeSelectIntegrationFromCache: Selector<{
    integrations: {
        cachedForms: IntegrationCachedFormData;
    };
}, Record<string, IntegrationFieldValue> | null, [integration: Integration]> & {
    unwrapped: (state: {
        cachedForms: IntegrationCachedFormData;
    }, integration: Integration) => Record<string, IntegrationFieldValue> | null;
}, checkValuesForChanges: Selector<{
    integrations: {
        cachedForms: IntegrationCachedFormData;
    };
}, (_accessors: string | string[], _value: IntegrationPrimitive) => false, [_integration: Integration]> & {
    unwrapped: (_state: {
        cachedForms: IntegrationCachedFormData;
    }, _integration: Integration) => (_accessors: string | string[], _value: IntegrationPrimitive) => false;
};
export {};
