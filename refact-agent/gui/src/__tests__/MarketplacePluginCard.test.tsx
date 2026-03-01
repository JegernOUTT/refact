import { describe, expect, test } from "vitest";
import { render, screen } from "../utils/test-utils";
import { MarketplacePluginCard } from "../features/Extensions/components/MarketplacePluginCard";
import type { PluginEntry } from "../services/refact/plugins";

const mockPlugin: PluginEntry = {
  id: "test-plugin",
  name: "Test Plugin",
  description: "A plugin for testing",
  marketplace: "test-marketplace",
};

describe("MarketplacePluginCard", () => {
  test("renders install button when not installed", () => {
    render(
      <MarketplacePluginCard plugin={mockPlugin} isInstalled={false} />,
    );
    expect(screen.getByRole("button", { name: /install/i })).toBeInTheDocument();
    expect(screen.queryByText(/Installed ✓/)).not.toBeInTheDocument();
  });

  test("renders installed state with uninstall button when installed", () => {
    render(
      <MarketplacePluginCard plugin={mockPlugin} isInstalled={true} />,
    );
    expect(screen.getByText(/Installed ✓/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /uninstall/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^install$/i })).not.toBeInTheDocument();
  });

  test("renders plugin name and description", () => {
    render(
      <MarketplacePluginCard plugin={mockPlugin} isInstalled={false} />,
    );
    expect(screen.getByText("Test Plugin")).toBeInTheDocument();
    expect(screen.getByText("A plugin for testing")).toBeInTheDocument();
  });

  test("renders marketplace badge", () => {
    render(
      <MarketplacePluginCard plugin={mockPlugin} isInstalled={false} />,
    );
    expect(screen.getByText("test-marketplace")).toBeInTheDocument();
  });
});
