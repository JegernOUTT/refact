import React from "react";
import { createRoot } from "react-dom/client";
import { Provider } from "react-redux";
import { setUpStore } from "../../src/app/store";
import { Theme } from "../../src/components/Theme";
import { BuddyDeskPreview } from "../../src/features/Buddy/preview/BuddyDeskPreview";
import { BuddyDesk } from "../../src/features/Buddy/BuddyDesk";
import { BuddyNativeDecisions } from "../../src/features/Buddy/BuddyNativeDecisions";
import { BuddyOperatorPlayPreview } from "../../src/features/Buddy/BuddyOperatorPlayPreview";
import { BuddyPresencePreviewProvider } from "../../src/features/Buddy/BuddyPresencePreviewProvider";
import { BuddyWorld } from "../../src/features/Buddy/BuddyWorld";
import { createInitialSemanticState } from "../../src/features/Buddy/state";
import { PALETTES, STAGES } from "../../src/features/Buddy/constants";
import "../../src/lib/render/web.css";

const root = document.getElementById("refact-chat");
const params = new URLSearchParams(window.location.search);
if (!root) throw new Error("Buddy Desk preview root is missing");
const store = setUpStore({
  config: {
    host: "web",
    dev: true,
    lspPort: 9,
    themeProps: {
      appearance: params.get("theme") === "light" ? "light" : "dark",
    },
  },
});
const connected = params.get("connected") === "1";
// Default-disabled prototype (plan B34/B35/B37). It is only mounted when the
// fixture URL explicitly opts in; it never activates production avatar polish.
const playPreviewEnabled = params.get("play") === "1";
const lowPower = params.get("lowpower") === "1";
const sceneState = { ...createInitialSemanticState(), name: "Pixel" };
const noAction = () => undefined;
function PlayFixture() {
  return (
    <BuddyPresencePreviewProvider
      connected={connected}
      lowPower={lowPower}
      textOnly={params.get("textonly") === "1"}
    >
      <BuddyWorld
        palette={PALETTES[sceneState.paletteIndex]}
        stage={STAGES[sceneState.progress.stage]}
        state={sceneState}
        pulse={null}
        pet={undefined}
        nowPlaying={null}
        activeQuest={null}
        activeSpeech={null}
        setupNeeded={false}
        onCanvasEvent={noAction}
        onCare={noAction}
        onOpenPage={noAction}
        onRunMode={noAction}
        onDismissSetup={noAction}
        onSpeechControl={noAction}
      />
      <BuddyOperatorPlayPreview enabled lowPower={lowPower} />
    </BuddyPresencePreviewProvider>
  );
}
createRoot(root).render(
  <Provider store={store}>
    <Theme>
      {connected ? (
        <>
          <BuddyDesk />
          <BuddyNativeDecisions />
        </>
      ) : (
        <BuddyDeskPreview />
      )}
      {playPreviewEnabled && <PlayFixture />}
    </Theme>
  </Provider>,
);
