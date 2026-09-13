import { describe, it, expect } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { BuddyArtifactBody } from "./BuddyArtifactBody";

describe("Buddy artifact body", () => {
  it("renders readable reports without allowing embedded actions, images or executable HTML", () => {
    const { container } = render(
      <BuddyArtifactBody
        body={
          "## Findings\n- **Authentication** failed\n\n[Execute](refact://run/action)\n\n![private](https://external.invalid/track)\n\n<script>alert(1)</script>\n\n```html\n<button>apply</button>\n```"
        }
      />,
    );
    expect(screen.getByRole("heading", { name: "Findings" })).toBeTruthy();
    expect(screen.getByText("Authentication")).toBeTruthy();
    expect(screen.getByText("Image omitted: private")).toBeTruthy();
    expect(container.querySelector("a,img,script,iframe,button")).toBeNull();
    expect(container.querySelector("pre")?.textContent).toContain(
      "<button>apply</button>",
    );
  });
});
