import React from "react";
import {
  Button,
  FieldStack,
  FieldText,
  FieldTextarea,
  FieldSwitch,
} from "../../../components/ui";
import {
  providersApi,
  type BedrockVerifyResponse,
  type BedrockProbeResponse,
} from "../../../services/refact/providers";
import styles from "./ProviderForm.module.css";

type Props = {
  providerName: string;
  readonly: boolean;
  savedProfile?: string;
  savedRegion?: string;
  settingsRevision?: string;
  recoverySettingsRevision?: string;
  recovery?: React.ReactNode;
  onSaveRecovery: (profile: string, region: string) => Promise<void>;
};

function errorMessage(error: unknown): string {
  if (error && typeof error === "object" && "data" in error) {
    const data: unknown = error.data;
    if (typeof data === "string") return data;
    if (data && typeof data === "object") {
      if ("detail" in data && typeof data.detail === "string")
        return data.detail;
      if ("error" in data && typeof data.error === "string") return data.error;
    }
  }
  return "Setup failed. Check the engine host's AWS credentials and permissions, then verify again. Expired or changed settings require a new verification.";
}

export const BedrockSetup: React.FC<Props> = ({
  providerName,
  readonly,
  savedProfile,
  savedRegion,
  settingsRevision,
  recoverySettingsRevision = settingsRevision,
  recovery,
  onSaveRecovery,
}) => {
  const {
    data: profiles,
    isError: profileError,
    refetch,
  } = providersApi.useGetBedrockProfilesQuery({ providerName });
  const [verify, verifyState] = providersApi.useVerifyBedrockMutation();
  const [probe, probeState] = providersApi.useProbeBedrockMutation();
  const [apply, applyState] = providersApi.useApplyBedrockMutation();
  const [profile, setProfile] = React.useState<string | null>(null);
  const [region, setRegion] = React.useState<string | null>(null);
  const [verified, setVerified] = React.useState<BedrockVerifyResponse>();
  const [tested, setTested] = React.useState<BedrockProbeResponse>();
  const [selected, setSelected] = React.useState<string[]>([]);
  const [custom, setCustom] = React.useState("");
  const [context1m, setContext1m] = React.useState(false);
  const [message, setMessage] = React.useState<string>();
  const [finished, setFinished] = React.useState(false);
  const [savingRecovery, setSavingRecovery] = React.useState(false);
  const [recoveryReady, setRecoveryReady] = React.useState<string>();
  const recoveryGeneration = React.useRef(0);
  const pendingRecovery = React.useRef<string>();
  const mounted = React.useRef(false);
  const generation = React.useRef(0);
  React.useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current += 1;
      recoveryGeneration.current += 1;
    };
  }, []);
  const activeProfile =
    profile ??
    (savedProfile?.trim()
      ? savedProfile.trim()
      : profiles?.suggested_profile ?? "");
  React.useEffect(() => {
    generation.current += 1;
    setVerified(undefined);
    setTested(undefined);
    setSelected([]);
    setContext1m(false);
  }, [settingsRevision, readonly, providerName, savedProfile, savedRegion]);
  const activeRegion =
    region ??
    (savedRegion?.trim() ? savedRegion.trim() : undefined) ??
    profiles?.profiles.find((p) => p.name === activeProfile)?.region ??
    profiles?.suggested_region ??
    "";
  const selectedPair = JSON.stringify([
    activeProfile.trim(),
    activeRegion.trim(),
  ]);
  const savedPair = JSON.stringify([savedProfile?.trim(), savedRegion?.trim()]);
  React.useEffect(() => {
    recoveryGeneration.current += 1;
    pendingRecovery.current = undefined;
    setRecoveryReady(undefined);
  }, [recoverySettingsRevision, readonly, providerName]);
  React.useEffect(() => {
    // A save may update the provider query before its promise resolves. Only
    // that exact requested pair is allowed through; other changes revoke it.
    if (pendingRecovery.current !== savedPair) {
      recoveryGeneration.current += 1;
      pendingRecovery.current = undefined;
      setRecoveryReady(undefined);
    }
  }, [savedPair]);
  const busy =
    verifyState.isLoading ||
    probeState.isLoading ||
    applyState.isLoading ||
    savingRecovery;
  const disabled = readonly || busy;
  const reset = () => {
    generation.current += 1;
    setVerified(undefined);
    setTested(undefined);
    setSelected([]);
    setContext1m(false);
    setMessage(undefined);
    recoveryGeneration.current += 1;
    pendingRecovery.current = undefined;
    setRecoveryReady(undefined);
  };
  const runVerify = async () => {
    reset();
    const requestGeneration = generation.current;
    try {
      const result = await verify({
        providerName,
        profile: activeProfile.trim(),
        region: activeRegion.trim(),
      }).unwrap();
      if (generation.current !== requestGeneration) return;
      setVerified(result);
      setSelected(result.candidates.slice(0, 8).map((c) => c.model_id));
    } catch (error) {
      if (generation.current === requestGeneration)
        setMessage(errorMessage(error));
    }
  };
  const ids = [
    ...new Set([
      ...selected,
      ...custom
        .split(/[,\n]/)
        .map((id) => id.trim())
        .filter(Boolean),
    ]),
  ];
  const prepareRecovery = async () => {
    reset();
    const requestGeneration = recoveryGeneration.current;
    pendingRecovery.current = selectedPair;
    // Keep the selected pair stable when the saved provider query refetches.
    setProfile(activeProfile.trim());
    setRegion(activeRegion.trim());
    setSavingRecovery(true);
    try {
      await onSaveRecovery(activeProfile.trim(), activeRegion.trim());
      if (recoveryGeneration.current === requestGeneration)
        setRecoveryReady(selectedPair);
    } catch {
      if (recoveryGeneration.current === requestGeneration)
        setMessage(
          "Couldn't save profile and region for SSO recovery. Check advanced settings before retrying.",
        );
    } finally {
      if (mounted.current) setSavingRecovery(false);
    }
  };
  const supports1m =
    ids.length > 0 &&
    ids.every(
      (id) =>
        verified?.candidates.some(
          (c) => c.model_id === id && c.supports_1m === true,
        ),
    );
  const runTest = async () => {
    if (!verified || !ids.length || ids.length > 8) return;
    const requestGeneration = generation.current;
    setMessage(undefined);
    setTested(undefined);
    try {
      const result = await probe({
        providerName,
        setup_id: verified.setup_id,
        model_ids: ids,
        context_1m: context1m,
      }).unwrap();
      if (generation.current !== requestGeneration) return;
      setTested(result);
      setSelected(
        result.results
          .filter((r) => r.status === "working")
          .map((r) => r.model_id),
      );
      setCustom("");
    } catch (error) {
      if (generation.current === requestGeneration)
        setMessage(errorMessage(error));
    }
  };
  const working = selected.filter(
    (id) =>
      tested?.results.some((r) => r.model_id === id && r.status === "working"),
  );
  const runApply = async () => {
    if (!verified || !tested || !working.length) return;
    const requestGeneration = generation.current;
    setMessage(undefined);
    try {
      const result = await apply({
        providerName,
        setup_id: verified.setup_id,
        config_revision: verified.config_revision,
        model_ids: working,
        context_1m: context1m,
      }).unwrap();
      if (generation.current !== requestGeneration) return;
      if (!result.success) throw new Error("Apply failed");
      setFinished(true);
      setMessage(
        "Working model IDs pinned. Global and project model defaults are unchanged.",
      );
    } catch (error) {
      if (generation.current === requestGeneration)
        setMessage(errorMessage(error));
    }
  };
  return (
    <section className={styles.formSection} aria-label="Set up Amazon Bedrock">
      <div className={styles.usageTitle}>Set up Amazon Bedrock</div>
      <div className={styles.credentialDescription}>
        Use AWS profiles on the engine host. Credentials never enter the
        browser. Verification checks identity and discovers Anthropic inference
        profiles without invoking models.
      </div>
      {finished ? (
        <>
          <div role="status" className={styles.usageMeta}>
            {message ??
              "Setup skipped. Existing models and defaults are unchanged."}
          </div>
          <Button
            variant="ghost"
            size="sm"
            disabled={disabled}
            onClick={() => {
              reset();
              setFinished(false);
            }}
          >
            Set up again
          </Button>
        </>
      ) : (
        <>
          <div className={styles.usageMeta}>1. Choose profile and region</div>
          <FieldStack
            label="AWS profile"
            control={
              <FieldText
                aria-label="AWS profile"
                list={`bedrock-profiles-${providerName}`}
                value={activeProfile}
                disabled={disabled}
                onChange={(value) => {
                  reset();
                  setProfile(value);
                  setRegion(
                    profiles?.profiles.find((p) => p.name === value)?.region ??
                      "",
                  );
                }}
              />
            }
          />
          <datalist id={`bedrock-profiles-${providerName}`}>
            {profiles?.profiles.map((p) => (
              <option key={p.name} value={p.name} />
            ))}
          </datalist>
          <FieldStack
            label="AWS region"
            control={
              <FieldText
                aria-label="AWS region"
                value={activeRegion}
                disabled={disabled}
                placeholder="Enter an AWS region"
                onChange={(value) => {
                  reset();
                  setRegion(value);
                }}
              />
            }
          />
          {profileError ? (
            <div role="alert" className={styles.usageMeta}>
              Couldn&apos;t read engine profiles. Enter a configured profile
              manually or{" "}
              <Button variant="ghost" size="sm" onClick={() => void refetch()}>
                Retry profiles
              </Button>
              .
            </div>
          ) : null}
          <div className={styles.fieldActions}>
            <Button
              size="sm"
              disabled={
                disabled || !activeProfile.trim() || !activeRegion.trim()
              }
              loading={verifyState.isLoading}
              onClick={() => void runVerify()}
            >
              Verify identity and discover models
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => {
                reset();
                setFinished(true);
              }}
            >
              Skip setup
            </Button>
          </div>
          {verified ? (
            <>
              <div role="status" className={styles.usageMeta}>
                Verified account {verified.identity.account} ·{" "}
                {verified.identity.arn}
              </div>
              {verified.warnings.map((warning, index) => (
                <div className={styles.usageMeta} key={index}>
                  {warning}
                </div>
              ))}
              <div className={styles.usageMeta}>2. Select models to test</div>
              <div className={styles.usageMeta}>
                {verified.inference_profiles.length} Anthropic inference
                profiles discovered. Discovery does not guarantee invocation
                permission.
              </div>
              <div className={styles.usageMeta}>
                Select up to 8 IDs per test. The first 8 are selected by
                default; deselect IDs to choose alternatives. Currently
                selected: {ids.length}.
              </div>
              {verified.candidates.map((candidate) => (
                <FieldStack
                  key={candidate.model_id}
                  label={candidate.model_id}
                  control={
                    <FieldSwitch
                      aria-label={candidate.model_id}
                      checked={selected.includes(candidate.model_id)}
                      disabled={disabled}
                      onChange={(checked) => {
                        setTested(undefined);
                        setContext1m(false);
                        setSelected((previous) =>
                          checked
                            ? [...previous, candidate.model_id]
                            : previous.filter(
                                (id) => id !== candidate.model_id,
                              ),
                        );
                      }}
                    />
                  }
                />
              ))}
              <FieldStack
                label="Alternative or custom model IDs"
                helper="One exact Bedrock model or inference-profile ID per line. Custom IDs must also pass testing."
                control={
                  <FieldTextarea
                    aria-label="Alternative or custom model IDs"
                    value={custom}
                    disabled={disabled}
                    onChange={(value) => {
                      setTested(undefined);
                      setContext1m(false);
                      setCustom(value);
                    }}
                  />
                }
              />
              {supports1m ? (
                <FieldStack
                  label="Enable 1M context"
                  helper="Only offered for models confirmed to support 1M context. Test checks request acceptance, not full context capacity."
                  control={
                    <FieldSwitch
                      aria-label="Enable 1M context"
                      checked={context1m}
                      disabled={disabled}
                      onChange={(value) => {
                        setTested(undefined);
                        setContext1m(value);
                      }}
                    />
                  }
                />
              ) : null}
              {!supports1m ? (
                <div className={styles.usageMeta}>
                  1M context is unavailable unless every selected model has
                  confirmed support.
                </div>
              ) : null}
              <div className={styles.usageMeta}>
                Test models sends a minimal request with a one-token output
                limit to each selected ID. AWS may charge for input and output
                tokens.
              </div>
              <Button
                size="sm"
                disabled={disabled || !ids.length || ids.length > 8}
                loading={probeState.isLoading}
                onClick={() => void runTest()}
              >
                Test models
              </Button>
              {tested ? (
                <>
                  <div className={styles.usageMeta}>3. Pin working IDs</div>
                  {tested.results.map((result) => (
                    <div key={result.model_id} className={styles.usageMeta}>
                      {result.model_id}:{" "}
                      {result.status === "working"
                        ? "Working"
                        : result.status === "denied"
                          ? "Permission denied"
                          : result.status === "unavailable"
                            ? "Unavailable"
                            : "Test failed"}
                      {result.message ? ` — ${result.message}` : ""}
                    </div>
                  ))}
                  {tested.warnings.map((warning, index) => (
                    <div className={styles.usageMeta} key={index}>
                      {warning}
                    </div>
                  ))}
                  <Button
                    size="sm"
                    disabled={disabled || !working.length}
                    loading={applyState.isLoading}
                    onClick={() => void runApply()}
                  >
                    Pin working model IDs
                  </Button>
                </>
              ) : null}
            </>
          ) : null}
          {message ? (
            <div role="alert" className={styles.usageMeta}>
              {message}
            </div>
          ) : null}
          {recovery ? (
            <details>
              <summary>Optional AWS SSO recovery</summary>
              <div className={styles.formSection}>
                <div className={styles.usageMeta}>
                  For an already configured SSO profile only. This action saves
                  the selected profile and region to provider settings and
                  invalidates earlier verification. After signing in, verify
                  identity again. To create a profile, run aws configure sso on
                  the engine host.
                </div>
                <Button
                  size="sm"
                  variant="soft"
                  disabled={
                    disabled || !activeProfile.trim() || !activeRegion.trim()
                  }
                  onClick={() => void prepareRecovery()}
                >
                  Save selected profile for SSO recovery
                </Button>
                {!readonly &&
                recoveryReady === selectedPair &&
                recoveryReady === savedPair
                  ? recovery
                  : null}
              </div>
            </details>
          ) : null}
        </>
      )}
    </section>
  );
};
