import { Columns2, MousePointer2, RefreshCw, SunMoon } from "lucide-react";

import {
  FieldSelect,
  FieldText,
  IconButton,
  Switch,
} from "../../components/ui";
import type {
  DesignSourceKind,
  DesignSurfaceState,
  DesignViewportPreset,
} from "./designSlice";
import styles from "./Design.module.css";

type DesignToolbarProps = {
  state: DesignSurfaceState;
  onPatch: (patch: Partial<DesignSurfaceState>) => void;
  onRefresh: () => void;
};

const sourceOptions = [
  { value: "live", label: "Live app" },
  { value: "artifact", label: "Artifact" },
  { value: "reference", label: "Reference" },
];

const viewportOptions = [
  { value: "375", label: "375" },
  { value: "768", label: "768" },
  { value: "1280", label: "1280" },
  { value: "1440", label: "1440" },
  { value: "custom", label: "Custom" },
];

export function DesignToolbar({
  onPatch,
  onRefresh,
  state,
}: DesignToolbarProps) {
  return (
    <header className={styles.toolbar} aria-label="Design toolbar">
      <FieldSelect
        aria-label="Design source"
        value={state.source}
        options={sourceOptions}
        onChange={(source) => onPatch({ source: source as DesignSourceKind })}
      />
      <FieldSelect
        aria-label="Viewport preset"
        value={state.viewportPreset}
        options={viewportOptions}
        onChange={(viewportPreset) =>
          onPatch({ viewportPreset: viewportPreset as DesignViewportPreset })
        }
      />
      {state.viewportPreset === "custom" ? (
        <FieldText
          aria-label="Custom viewport width"
          className={styles.numberInput}
          min={240}
          type="number"
          value={String(state.customWidth)}
          onChange={(value) =>
            onPatch({ customWidth: Math.max(240, Number(value) || 240) })
          }
        />
      ) : null}
      <FieldSelect
        aria-label="Device pixel ratio"
        value={String(state.devicePixelRatio)}
        options={[
          { value: "1", label: "DPR 1" },
          { value: "2", label: "DPR 2" },
          { value: "3", label: "DPR 3" },
        ]}
        onChange={(value) => onPatch({ devicePixelRatio: Number(value) })}
      />
      <FieldSelect
        aria-label="Design zoom"
        value={String(state.zoom)}
        options={[50, 75, 100, 125, 150, 200].map((value) => ({
          value: String(value),
          label: `${value}%`,
        }))}
        onChange={(value) => onPatch({ zoom: Number(value) })}
      />
      <IconButton
        aria-label={`Use ${state.theme === "light" ? "dark" : "light"} theme`}
        icon={SunMoon}
        onClick={() =>
          onPatch({ theme: state.theme === "light" ? "dark" : "light" })
        }
        size="sm"
        variant="ghost"
      />
      <IconButton
        aria-label="Toggle element picker"
        disabled={state.liveStatus !== "interactive"}
        icon={MousePointer2}
        onClick={() => onPatch({ pickerEnabled: !state.pickerEnabled })}
        size="sm"
        variant={state.pickerEnabled ? "primary" : "ghost"}
      />
      <IconButton
        aria-label="Toggle reference comparison"
        icon={Columns2}
        onClick={() =>
          onPatch({
            compareMode:
              state.compareMode === "side-by-side" ? "overlay" : "side-by-side",
          })
        }
        size="sm"
        variant={state.compareMode === "overlay" ? "primary" : "ghost"}
      />
      <Switch
        aria-label="Reference overlay"
        checked={state.compareMode === "overlay"}
        onCheckedChange={(checked) =>
          onPatch({ compareMode: checked ? "overlay" : "side-by-side" })
        }
      />
      {state.compareMode === "overlay" ? (
        <FieldSelect
          aria-label="Reference opacity"
          value={String(state.overlayOpacity)}
          options={[25, 50, 75].map((value) => ({
            value: String(value),
            label: `${value}%`,
          }))}
          onChange={(value) => onPatch({ overlayOpacity: Number(value) })}
        />
      ) : null}
      <IconButton
        aria-label="Refresh design surface"
        icon={RefreshCw}
        onClick={onRefresh}
        size="sm"
        variant="ghost"
      />
    </header>
  );
}
