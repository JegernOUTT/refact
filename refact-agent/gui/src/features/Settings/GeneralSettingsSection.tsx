import React, { useCallback, useState } from "react";
import { Keyboard } from "lucide-react";

import {
  useAppDispatch,
  useAppSelector,
  useEventsBusForIDE,
} from "../../hooks";
import { useAppearance } from "../../hooks/useAppearance";
import {
  selectConfig,
  selectThemeMode,
  setThemeMode,
} from "../Config/configSlice";
import { push } from "../Pages/pagesSlice";
import {
  useGetProjectInformationQuery,
  useSaveProjectInformationMutation,
} from "../../services/refact/projectInformation";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import {
  useGetSkillsSettingsQuery,
  useSaveSkillsSettingsMutation,
} from "../../services/refact/skillsSettings";
import { Button, FieldSwitch, Select, SettingItem } from "../../components/ui";
import { SettingsGroup, SettingsSection } from "./SettingsSection";
import styles from "./GeneralSettingsSection.module.css";

type SkillsMode = "index_only" | "inject_full" | "off";
type OverviewState = "Healthy" | "Indexing" | "Off" | "Error";

function codeGraphState(
  isError: boolean,
  alive?: string,
  state?: string,
): OverviewState {
  if (isError || state === "error") return "Error";
  if (state === "indexing" || alive === "indexing") return "Indexing";
  if (state === "turned_off" || alive === "turned_off" || alive === "") {
    return "Off";
  }
  if (state === "working" || alive === "working") return "Healthy";
  return "Error";
}

function semanticIndexState(
  isError: boolean,
  alive?: string,
  state?: string,
  hasError?: boolean,
): OverviewState {
  if (isError || hasError) return "Error";
  if (alive === "turned_off" || alive === "" || alive === "false") return "Off";
  if (state === "starting" || state === "parsing") return "Indexing";
  if (
    state === "done" ||
    state === "cooldown" ||
    alive === "working" ||
    alive === "true"
  ) {
    return "Healthy";
  }
  return "Error";
}

export const GeneralSettingsSection: React.FC = () => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const themeMode = useAppSelector(selectThemeMode);
  const { openHotKeys, openSettings } = useEventsBusForIDE();
  const { appearance } = useAppearance();

  const {
    data: projectInformation,
    isLoading: isProjectInformationLoading,
    isError: isProjectInformationError,
  } = useGetProjectInformationQuery(undefined);
  const [saveProjectInformation, { isLoading: isSavingProjectInformation }] =
    useSaveProjectInformationMutation();
  const [projectInformationSaveError, setProjectInformationSaveError] =
    useState<string | null>(null);
  const {
    data: skillsSettings,
    isLoading: isSkillsLoading,
    isError: isSkillsError,
  } = useGetSkillsSettingsQuery(undefined);
  const [saveSkillsSettings, { isLoading: isSavingSkills }] =
    useSaveSkillsSettingsMutation();
  const [skillsSaveError, setSkillsSaveError] = useState<string | null>(null);
  const { data: ragStatus, isError: isRagStatusError } = useGetRagStatusQuery(
    undefined,
    { pollingInterval: 5000 },
  );

  const handleAppearanceChange = useCallback(
    (value: string) => {
      dispatch(setThemeMode(value as "light" | "dark" | "inherit"));
    },
    [dispatch],
  );

  const handleProjectInformationChange = useCallback(
    (enabled: boolean) => {
      if (!projectInformation) return;
      setProjectInformationSaveError(null);
      void saveProjectInformation({ ...projectInformation, enabled })
        .unwrap()
        .catch(() => {
          setProjectInformationSaveError(
            "Could not save project context. Open a project and try again.",
          );
        });
    },
    [projectInformation, saveProjectInformation],
  );

  const handleSkillsChange = useCallback(
    (autoTrigger: string) => {
      if (!skillsSettings) return;
      setSkillsSaveError(null);
      void saveSkillsSettings({
        ...skillsSettings,
        auto_trigger: autoTrigger as SkillsMode,
      })
        .unwrap()
        .catch(() => {
          setSkillsSaveError("Could not save skill activation. Try again.");
        });
    },
    [saveSkillsSettings, skillsSettings],
  );

  const hostLabel =
    config.host === "vscode"
      ? "Extension Settings"
      : config.host === "jetbrains"
        ? "Plugin Settings"
        : null;

  const codeGraph = codeGraphState(
    isRagStatusError,
    ragStatus?.codegraph_alive,
    ragStatus?.codegraph?.state,
  );
  const semanticIndex = semanticIndexState(
    isRagStatusError,
    ragStatus?.vecdb_alive,
    ragStatus?.vecdb?.state,
    Boolean(ragStatus?.vec_db_error),
  );

  return (
    <SettingsSection
      title="General"
      description="Manage session appearance, project-specific context, indexing, and host integration."
    >
      <SettingsGroup title="Appearance">
        <SettingItem
          className="rf-enter"
          title="Theme preview"
          description="Preview light, dark, or inherited appearance for this session. The host does not currently provide a persistence or write-back contract."
          control={
            <Select
              value={themeMode ?? appearance}
              onValueChange={handleAppearanceChange}
            >
              <Select.Trigger
                className={styles.select}
                aria-label="Theme preview"
              />
              <Select.Content>
                <Select.Item value="dark">Dark</Select.Item>
                <Select.Item value="light">Light</Select.Item>
                <Select.Item value="inherit">Inherit</Select.Item>
              </Select.Content>
            </Select>
          }
        />
      </SettingsGroup>

      <SettingsGroup title="Project context">
        <SettingItem
          className="rf-enter"
          title="Inject project information"
          description={
            isProjectInformationError
              ? "Project information is unavailable. Open a project and try again."
              : projectInformationSaveError ??
                "Controls whether project information can be injected into chats. The project information control in the composer remains a per-chat choice."
          }
          control={
            <FieldSwitch
              aria-label="Inject project information"
              checked={projectInformation?.enabled ?? false}
              onChange={handleProjectInformationChange}
              disabled={
                isProjectInformationLoading ||
                isSavingProjectInformation ||
                isProjectInformationError ||
                !projectInformation
              }
            />
          }
        />
      </SettingsGroup>

      <SettingsGroup
        title="Skills"
        description="This setting belongs to the active project."
      >
        <SettingItem
          className="rf-enter"
          title="Automatic skill context"
          description={
            isSkillsError
              ? "Skills settings are unavailable. Open an active project and try again."
              : skillsSaveError ??
                "Index only adds compact skill names and descriptions; inject full adds matching skill instructions and uses more tokens. Off adds no automatic skill context."
          }
          control={
            <Select
              value={skillsSettings?.auto_trigger}
              onValueChange={handleSkillsChange}
              disabled={
                isSkillsLoading ||
                isSavingSkills ||
                isSkillsError ||
                !skillsSettings
              }
            >
              <Select.Trigger
                className={styles.select}
                aria-label="Automatic skill context"
                placeholder={isSkillsLoading ? "Loading…" : "Unavailable"}
              />
              <Select.Content>
                <Select.Item value="index_only">Index only</Select.Item>
                <Select.Item value="inject_full">Inject full</Select.Item>
                <Select.Item value="off">Off</Select.Item>
              </Select.Content>
            </Select>
          }
        />
      </SettingsGroup>

      <SettingsGroup title="Indexing overview">
        <SettingItem
          className="rf-enter"
          title="Index health"
          description={`CodeGraph: ${codeGraph} · Semantic index: ${semanticIndex}`}
          control={
            <Button
              variant="soft"
              onClick={() => dispatch(push({ name: "indexing settings" }))}
            >
              Configure indexing
            </Button>
          }
        />
      </SettingsGroup>

      {(hostLabel ?? config.currentWorkspaceName) && (
        <SettingsGroup
          title="Runtime and host"
          description="Information reported by the current host session and shortcuts managed by your IDE."
        >
          {config.currentWorkspaceName && (
            <SettingItem
              className="rf-enter"
              title="Active workspace"
              description={config.currentWorkspaceName}
            />
          )}
          {hostLabel && (
            <>
              <SettingItem
                className="rf-enter"
                title="Host settings"
                description={`Open Refact settings managed by ${config.host}.`}
                control={
                  <Button variant="soft" onClick={openSettings}>
                    {hostLabel}
                  </Button>
                }
              />
              <SettingItem
                className="rf-enter"
                title="IDE hotkeys"
                description="Open the host keyboard shortcuts for Refact commands."
                control={
                  <Button
                    variant="soft"
                    leftIcon={Keyboard}
                    onClick={openHotKeys}
                  >
                    IDE Hotkeys
                  </Button>
                }
              />
            </>
          )}
        </SettingsGroup>
      )}
    </SettingsSection>
  );
};
