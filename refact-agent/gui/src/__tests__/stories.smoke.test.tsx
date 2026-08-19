import type { ComponentType } from "react";
import { cleanup, render } from "@testing-library/react";
import { composeStories, setProjectAnnotations } from "@storybook/react";
import type { RequestHandler } from "msw";
import { afterEach, describe, expect, it, vi } from "vitest";

// preview.tsx initializes the browser-only MSW worker at module evaluation time.
// Hoisting this mock lets us reuse the real preview decorators/globalTypes in node;
// only the worker initialization and loader are replaced with node-safe no-ops.
vi.mock("msw-storybook-addon", () => ({
  initialize: vi.fn(),
  mswLoader: vi.fn(() => ({})),
}));

import * as previewAnnotations from "../../.storybook/preview";
import * as toolCardsFileOpsStories from "../components/ChatContent/ToolCardsFileOps.stories";
import * as toolCardsAgenticStories from "../components/ChatContent/ToolCardsAgentic.stories";
import * as transcriptElementsStories from "../components/ChatContent/TranscriptElements.stories";
import * as chatContentStories from "../components/ChatContent/ChatContent.stories";
import * as chatFormStories from "../components/ChatForm/ChatForm.stories";
import * as retryFormStories from "../components/ChatForm/RetryForm.stories";
import * as taskProgressWidgetStories from "../components/TaskProgressWidget/TaskProgressWidget.stories";
import * as planBannerStories from "../components/ChatContent/PlanBanner/PlanBanner.stories";
import * as chatShieldStories from "../features/Privacy/ChatShield.stories";
import * as modeSelectStories from "../components/ChatForm/ModeSelect.stories";
import * as chatSettingsDropdownStories from "../components/ChatForm/ChatSettingsDropdown.stories";
import * as deletePopoverStories from "../components/DeletePopover/DeletePopover.stories";
import * as createWorktreeModalStories from "../features/Worktrees/CreateWorktreeModal.stories";
import * as modeTransitionDialogStories from "../components/ChatForm/ModeTransitionDialog.stories";
import * as taskPlannerDialogStories from "../components/ChatForm/TaskPlannerDialog.stories";
import * as mcpImportDialogStories from "../components/IntegrationsView/MCPImportDialog.stories";
import * as trajectoryStories from "../components/Trajectory/Trajectory.stories";
import * as threadInfoButtonStories from "../components/Buttons/ThreadInfoButton.stories";
import * as checkpointsStories from "../features/Checkpoints/Checkpoints.stories";
import * as errorCalloutStories from "../components/Callout/ErrorCallout.stories";
import * as toolConfirmationStories from "../components/ChatForm/ToolConfirmation.stories";
import * as addCustomModelModalStories from "../features/Providers/ProviderForm/ProviderModelsList/AddCustomModelModal.stories";
import { server } from "../utils/mockServer";

setProjectAnnotations(previewAnnotations.default);

type AssertionTier = "non-empty" | "no-crash";

const COVERED_STORY_FILES = [
  "../components/Buttons/ThreadInfoButton.stories.tsx",
  "../components/Callout/ErrorCallout.stories.tsx",
  "../components/ChatContent/ChatContent.stories.tsx",
  "../components/ChatContent/PlanBanner/PlanBanner.stories.tsx",
  "../components/ChatContent/ToolCardsAgentic.stories.tsx",
  "../components/ChatContent/ToolCardsFileOps.stories.tsx",
  "../components/ChatContent/TranscriptElements.stories.tsx",
  "../components/ChatForm/ChatForm.stories.tsx",
  "../components/ChatForm/ChatSettingsDropdown.stories.tsx",
  "../components/ChatForm/ModeSelect.stories.tsx",
  "../components/ChatForm/ModeTransitionDialog.stories.tsx",
  "../components/ChatForm/RetryForm.stories.tsx",
  "../components/ChatForm/TaskPlannerDialog.stories.tsx",
  "../components/ChatForm/ToolConfirmation.stories.tsx",
  "../components/DeletePopover/DeletePopover.stories.tsx",
  "../components/IntegrationsView/MCPImportDialog.stories.tsx",
  "../components/TaskProgressWidget/TaskProgressWidget.stories.tsx",
  "../components/Trajectory/Trajectory.stories.tsx",
  "../features/Checkpoints/Checkpoints.stories.tsx",
  "../features/Privacy/ChatShield.stories.tsx",
  "../features/Providers/ProviderForm/ProviderModelsList/AddCustomModelModal.stories.tsx",
  "../features/Worktrees/CreateWorktreeModal.stories.tsx",
] as const;

const EXCLUDED_STORY_FILES = [
  // Legacy primitive gallery; covered by focused component tests.
  "../components/Accordion/Accordion.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/Callout/Callout.stories.tsx",
  // Full application chat scenario is too broad for this smoke suite.
  "../components/Chat/Chat.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/ChatLinks/ChatLinks.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/Checkbox/Checkbox.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/Collapsible/Collapsible.stories.tsx",
  // Interactive combobox requires dedicated interaction coverage.
  "../components/ComboBox/ComboBox.stories.tsx",
  // Visual animation gallery is not useful in happy-dom.
  "../components/LogoAnimation/LogoAnimation.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/Reveal/Reveal.stories.tsx",
  // Layout-dependent scroll gallery is not stable in happy-dom.
  "../components/ScrollArea/ScrollArea.stories.tsx",
  // Layout-dependent scroll gallery is not stable in happy-dom.
  "../components/ScrollArea/ScrollAreaWithAnchor.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/Select/Select.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/Spinner/Spinner.stories.tsx",
  // Visual animation gallery is not useful in happy-dom.
  "../components/Text/AnimatedText.stories.tsx",
  // Legacy primitive gallery; covered by focused component tests.
  "../components/TextArea/TextArea.stories.tsx",
  // Usage displays depend on runtime token telemetry fixtures.
  "../components/UsageCounter/UsageCounter.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Badge/Badge.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Button/Button.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Card/Card.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Chip/Chip.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Combobox/Combobox.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/DataTable/DataTable.stories.tsx",
  // Aggregate design-system gallery is visual documentation.
  "../components/ui/DesignSystemOverview.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Dialog/Dialog.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/EditableTable/EditableTable.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/EmptyState/EmptyState.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/ErrorState/ErrorState.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Field/Field.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Icon/Icon.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/LoadingState/LoadingState.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Menu/Menu.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/ModelSelector/ModelSelector.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Popover/Popover.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/SegmentedControl/SegmentedControl.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Select/Select.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Sheet/Sheet.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Skeleton/Skeleton.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Slider/Slider.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/StatusDot/StatusDot.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Surface/Surface.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Switch/Switch.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Tabs/Tabs.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/ToolCard/ToolCard.stories.tsx",
  // UI-kit visual gallery is reviewed in Storybook.
  "../components/ui/Tooltip/Tooltip.stories.tsx",
  // Virtualized UI-kit gallery requires measurable browser layout.
  "../components/ui/VirtualList/VirtualList.stories.tsx",
  // Virtualized UI-kit gallery requires measurable browser layout.
  "../components/ui/VirtualizedGrid/VirtualizedGrid.stories.tsx",
  // Login page behavior is covered by feature-level tests.
  "../features/Login/LoginPage.stories.tsx",
] as const;

type SmokeStory = ComponentType & {
  parameters?: {
    msw?: {
      handlers?: unknown;
    };
  };
  play?: (context: { canvasElement: HTMLElement }) => void | Promise<void>;
};

type SmokeCase = {
  name: string;
  story: SmokeStory;
  assertion: AssertionTier;
};

function casesFrom(
  moduleName: string,
  composed: object,
  assertion: AssertionTier = "non-empty",
): SmokeCase[] {
  return Object.entries(composed).map(([storyName, story]) => ({
    name: `${moduleName}/${storyName}`,
    story: story as SmokeStory,
    assertion,
  }));
}

function flattenHandlers(value: unknown): RequestHandler[] {
  if (Array.isArray(value)) {
    return value.flatMap(flattenHandlers);
  }

  return value == null ? [] : [value as RequestHandler];
}

const smokeCases: SmokeCase[] = [
  ...casesFrom("ToolCardsFileOps", composeStories(toolCardsFileOpsStories)),
  ...casesFrom("ToolCardsAgentic", composeStories(toolCardsAgenticStories)),
  ...casesFrom("TranscriptElements", composeStories(transcriptElementsStories)),
  // Portals and virtualized content may not populate the RTL container in
  // happy-dom. The no-crash tier still verifies composition, decorators,
  // effects, MSW setup, play calls, and that rendering leaves a body tree.
  ...casesFrom("ChatContent", composeStories(chatContentStories), "no-crash"),
  ...casesFrom("ChatForm", composeStories(chatFormStories)),
  ...casesFrom("RetryForm", composeStories(retryFormStories)),
  ...casesFrom("TaskProgressWidget", composeStories(taskProgressWidgetStories)),
  ...casesFrom("PlanBanner", composeStories(planBannerStories)),
  ...casesFrom("ChatShield", composeStories(chatShieldStories)),
  ...casesFrom("ModeSelect", composeStories(modeSelectStories)),
  ...casesFrom(
    "ChatSettingsDropdown",
    composeStories(chatSettingsDropdownStories),
  ),
  ...casesFrom("DeletePopover", composeStories(deletePopoverStories)),
  // Radix modal content is rendered into document.body through a portal, so
  // the RTL root container itself may remain empty even when rendering works.
  ...casesFrom(
    "CreateWorktreeModal",
    composeStories(createWorktreeModalStories),
    "no-crash",
  ),
  ...casesFrom(
    "ModeTransitionDialog",
    composeStories(modeTransitionDialogStories),
    "no-crash",
  ),
  ...casesFrom(
    "TaskPlannerDialog",
    composeStories(taskPlannerDialogStories),
    "no-crash",
  ),
  ...casesFrom(
    "MCPImportDialog",
    composeStories(mcpImportDialogStories),
    "no-crash",
  ),
  ...casesFrom(
    "AddCustomModelModal",
    composeStories(addCustomModelModalStories),
    "no-crash",
  ),
  ...casesFrom("Trajectory", composeStories(trajectoryStories)),
  ...casesFrom("ThreadInfoButton", composeStories(threadInfoButtonStories)),
  ...casesFrom("Checkpoints", composeStories(checkpointsStories)),
  ...casesFrom("ErrorCallout", composeStories(errorCalloutStories)),
  ...casesFrom("ToolConfirmation", composeStories(toolConfirmationStories)),
];

describe("Storybook story smoke tests", () => {
  afterEach(() => {
    cleanup();
    server.resetHandlers();
  });

  it.each(smokeCases)("$name", async ({ name, story: Story, assertion }) => {
    const handlers = flattenHandlers(Story.parameters?.msw?.handlers);
    if (handlers.length > 0) {
      server.use(...handlers);
    }

    let rendered: ReturnType<typeof render> | undefined;
    expect(() => {
      rendered = render(<Story />);
    }, `${name} threw while rendering`).not.toThrow();
    if (!rendered) {
      throw new Error(`${name} did not return a render result`);
    }
    const { container } = rendered;

    if (assertion === "non-empty") {
      expect(
        container.childNodes.length,
        `${name} rendered an empty container`,
      ).toBeGreaterThan(0);
    } else {
      expect(
        document.body.childElementCount,
        `${name} rendered no document body children`,
      ).toBeGreaterThan(0);
    }

    if (Story.play) {
      try {
        await Story.play({ canvasElement: container });
      } catch (error: unknown) {
        const detail = error instanceof Error ? error.message : String(error);
        throw new Error(`${name} play function failed: ${detail}`);
      }
    }
  });

  it("classifies every story file for smoke coverage", () => {
    const storyModules = import.meta.glob("../**/*.stories.@(ts|tsx)", {
      eager: false,
    });
    const classified = new Set<string>([
      ...COVERED_STORY_FILES,
      ...EXCLUDED_STORY_FILES,
    ]);
    const unclassified = Object.keys(storyModules).filter(
      (path) => !classified.has(path),
    );

    expect(unclassified, "Unclassified Storybook story files").toEqual([]);
  });
});
