import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StructuredValue } from "./structured-value";

describe("StructuredValue", () => {
  it("renders nested values without serialized JSON", () => {
    const { container } = render(<StructuredValue value={{ ready: ["gene_expression_by_sample", "survival_expression"], policy: { public: true, retries: 3 } }} />);
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(screen.getByText("gene_expression_by_sample")).toBeInTheDocument();
    expect(screen.getByText("Yes")).toBeInTheDocument();
    expect(container.textContent).not.toContain('{"ready"');
  });

  it("shows the requested empty state", () => {
    render(<StructuredValue value={{}} emptyLabel="Nothing reported." />);
    expect(screen.getByText("Nothing reported.")).toBeInTheDocument();
  });
});
