import React from "react";
import { createRoot } from "react-dom/client";
import { AtCommandChip } from "../../src/components/AtCommands/AtCommandChip";
import "../../src/styles/tokens.css";
import "../../src/lib/render/web.css";

const root = document.getElementById("root");
if (!root) throw new Error("AtCommandChip interaction test root is missing");

export function ChipInteractionFixture() {
  const [disabledCalls, setDisabledCalls] = React.useState(0);
  const [enabledCalls, setEnabledCalls] = React.useState(0);

  return (
    <main>
      <button>Before chips</button>
      <AtCommandChip
        chip={{ type: "file", label: "Disabled file", disabled: true }}
        onClick={() => setDisabledCalls((count) => count + 1)}
      />
      <AtCommandChip
        chip={{ type: "file", label: "Enabled file", disabled: false }}
        onClick={() => setEnabledCalls((count) => count + 1)}
      />
      <button>After chips</button>
      <output data-testid="disabled-calls">{disabledCalls}</output>
      <output data-testid="enabled-calls">{enabledCalls}</output>
    </main>
  );
}

createRoot(root).render(<ChipInteractionFixture />);
