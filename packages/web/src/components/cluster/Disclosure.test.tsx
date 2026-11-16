import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Disclosure } from "./Disclosure";

afterEach(cleanup);

describe("Disclosure", () => {
  it("mounts its content only while open", () => {
    const content = vi.fn(() => <p>Inside</p>);
    render(<Disclosure id="part" label="More settings" description="Rarely changed.">{content}</Disclosure>);

    const toggle = screen.getByRole("button", { name: /More settings/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(toggle.getAttribute("aria-controls")).toBe("part");
    expect(screen.getByText("Rarely changed.")).toBeTruthy();
    expect(screen.queryByText("Inside")).toBeNull();
    expect(content).not.toHaveBeenCalled();

    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Inside")).toBeTruthy();

    fireEvent.click(toggle);
    expect(screen.queryByText("Inside")).toBeNull();
  });
});
