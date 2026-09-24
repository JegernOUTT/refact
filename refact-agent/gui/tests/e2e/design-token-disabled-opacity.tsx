import React from "react";
import { createRoot } from "react-dom/client";
import { AtCommandChip } from "../../src/components/AtCommands/AtCommandChip";
import "../../src/styles/tokens.css";
import "../../src/lib/render/web.css";

const root = document.getElementById("root");
if (!root) throw new Error("Design token test root is missing");

createRoot(root).render(
  <main>
    <AtCommandChip
      chip={{ type: "file", label: "Disabled file", disabled: true }}
    />
    <AtCommandChip
      chip={{ type: "file", label: "Enabled file", disabled: false }}
    />
  </main>,
);
