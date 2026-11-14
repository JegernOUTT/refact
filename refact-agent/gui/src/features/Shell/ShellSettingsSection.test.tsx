import { HttpResponse, http } from "msw";
import { beforeEach, describe, expect, it } from "vitest";

import { STUB_SHELL_POLICY } from "../../__fixtures__/shellPolicy";
import { server } from "../../utils/mockServer";
import { render, screen } from "../../utils/test-utils";
import { ShellSettingsSection } from "./ShellSettingsSection";

const POLICY_URL = "*/v1/shell-policy";
const UNEXPECTED_FORMAT =
  "The engine returned a shell policy in an unexpected format.";

beforeEach(() => {
  server.use(
    http.get("*/v1/shell-policy/audit", () =>
      HttpResponse.json({ entries: [] }),
    ),
  );
});

describe("ShellSettingsSection", () => {
  it("renders the engine policy without a foreground timeout field", async () => {
    server.use(
      http.get(POLICY_URL, () => HttpResponse.json(STUB_SHELL_POLICY)),
    );

    render(<ShellSettingsSection />);

    expect(
      await screen.findByRole("combobox", { name: "Approval mode" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("spinbutton", { name: "Output line limit" }),
    ).toHaveValue(40);
    expect(
      screen.queryByLabelText("Foreground timeout seconds"),
    ).not.toBeInTheDocument();
  });

  it("explains an invalid policy in a successful JSON response", async () => {
    server.use(
      http.get(POLICY_URL, () =>
        HttpResponse.json({ ...STUB_SHELL_POLICY, execution: {} }),
      ),
    );

    render(<ShellSettingsSection />);

    expect(await screen.findByText(UNEXPECTED_FORMAT)).toBeInTheDocument();
  });

  it("explains a successful response that is not JSON", async () => {
    server.use(
      http.get(POLICY_URL, () => HttpResponse.html("<!doctype html><html />")),
    );

    render(<ShellSettingsSection />);

    expect(await screen.findByText(UNEXPECTED_FORMAT)).toBeInTheDocument();
  });

  it("reports the HTTP status of a JSON error response", async () => {
    server.use(
      http.get(POLICY_URL, () =>
        HttpResponse.json({ detail: "Internal server error" }, { status: 500 }),
      ),
    );

    render(<ShellSettingsSection />);

    expect(
      await screen.findByText("The shell policy endpoint returned HTTP 500."),
    ).toBeInTheDocument();
  });

  it("reports the HTTP status of a non-JSON error response", async () => {
    server.use(
      http.get(POLICY_URL, () =>
        HttpResponse.html("<!doctype html><title>Not Found</title>", {
          status: 404,
        }),
      ),
    );

    render(<ShellSettingsSection />);

    expect(
      await screen.findByText("The shell policy endpoint returned HTTP 404."),
    ).toBeInTheDocument();
  });

  it("reports an unreachable endpoint on a network error", async () => {
    server.use(http.get(POLICY_URL, () => HttpResponse.error()));

    render(<ShellSettingsSection />);

    expect(
      await screen.findByText(
        "The shell policy endpoint could not be reached.",
      ),
    ).toBeInTheDocument();
  });
});
