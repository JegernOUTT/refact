import "@radix-ui/themes/styles.css";
import "../../src/styles/tokens.css";
import "../../src/styles/base.css";
import "../../src/styles/glass.css";
import "../../src/styles/motion.css";
import "../../src/styles/responsive.css";
import "../../src/styles/scrollbar.css";
import "../../src/components/Theme/theme-config.css";
import "../../src/components/shared/tokens.css";

import React from "react";
import { createRoot } from "react-dom/client";
import { Theme } from "@radix-ui/themes";
import { ToolCard } from "../../src/components/ui/ToolCard/ToolCard";
import { CompactToolDetails } from "../../src/components/Tools/AgenticTool/CompactToolDetails";
import { PagedText } from "../../src/components/Tools/AgenticTool/PagedText";

const fallback = new URLSearchParams(window.location.search).has("fallback");
if (fallback) {
  const setAttribute = Element.prototype.setAttribute;
  const removeAttribute = Element.prototype.removeAttribute;
  Element.prototype.setAttribute = function (name, value) {
    setAttribute.call(this, name === "inert" ? "data-test-inert" : name, value);
  };
  Element.prototype.removeAttribute = function (name) {
    removeAttribute.call(this, name === "inert" ? "data-test-inert" : name);
  };
}

function FocusFixture() {
  const [open, setOpen] = React.useState(false);
  const [mounted, setMounted] = React.useState(true);
  const [added, setAdded] = React.useState(false);
  return (
    <Theme appearance="light" data-host="web">
      <main data-focus-mode={fallback ? "fallback" : "native"}>
        <button onClick={() => setOpen((value) => !value)}>
          Toggle outer externally
        </button>
        <button onClick={() => setAdded(true)}>Add control</button>
        <button onClick={() => setMounted((value) => !value)}>
          {mounted ? "Unmount cards" : "Mount cards"}
        </button>
        <button>Before cards</button>
        {mounted && (
          <ToolCard
            title="Outer receipt"
            density="compact"
            open={open}
            onOpenChange={setOpen}
          >
            <CompactToolDetails
              facts={[{ label: "Request", value: "Local focus fixture" }]}
              navigation={<button>Related navigation</button>}
            >
              <input aria-label="Notes" defaultValue="Keep this note" />
              <PagedText
                label="Full response"
                body={`${"A".repeat(4096)}END`}
              />
              <button>Inner action</button>
              {added && <button>Added action</button>}
            </CompactToolDetails>
          </ToolCard>
        )}
        <button>After cards</button>
      </main>
    </Theme>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Compact focus root is missing");
createRoot(root).render(<FocusFixture />);
