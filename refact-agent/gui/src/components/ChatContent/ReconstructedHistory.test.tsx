import { describe, it, expect } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { SummarizationMessage } from "./SummarizationMessage";
import { syntheticCompressionReportMessage } from "../../services/refact/types";

describe("reconstructed history disclosure", () => {
  it("retains flattened metadata and renders payload inside a collapsed report", async () => {
    const message = syntheticCompressionReportMessage({
      role: "compression_report",
      content: "Rebuilt",
      compression_report: {
        kind: "reconstructed_history",
        schema_version: 1,
        payload: {
          messages: [
            {
              role: "user",
              content: "Reconstructed request",
              message_id: "payload-1",
            },
          ],
        },
      },
    });
    const { user } = render(<SummarizationMessage message={message} />);
    const disclosure = screen.getByTestId("reconstructed-history-report");
    expect(disclosure).not.toHaveAttribute("open");
    await user.click(screen.getByText(/Context rebuilt/));
    expect(disclosure).toHaveAttribute("open");
    expect(screen.getAllByText("Reconstructed request")).toHaveLength(1);
  });
});
