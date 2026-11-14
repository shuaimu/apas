import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { AutopilotSpec } from "@apas/protocol";
import {
  autopilotTemplateById,
  autopilotTemplateEndingNote,
  AUTOPILOT_OWN_TEXT,
  AUTOPILOT_OWN_TEXT_DESCRIPTION,
  AUTOPILOT_OWN_TEXT_KEPT_TEXT,
  AUTOPILOT_TEMPLATES,
  DEFAULT_AUTOPILOT_TEMPLATE,
} from "@/lib/autopilotTemplates";
import { AutopilotStartDialog } from "./AutopilotStartDialog";

/**
 * The start dialog's prompt templates. What a start sends, its validation
 * and the rest of the dialog are covered through the control that opens it
 * (`AutopilotControl.test.tsx`) and the phone screen.
 */

const TODO = autopilotTemplateById("todo")!;
const WATCH = autopilotTemplateById("tests_green")!;

const LAST: AutopilotSpec = {
  prompt: "Check the build and fix what fails.",
  repeat: { kind: "every", seconds: 900 },
  iteration_limit: { kind: "at_most", value: 20 },
  time_limit: { kind: "none" },
  end_without_tool_call: false,
};

function open(props: Partial<Parameters<typeof AutopilotStartDialog>[0]> = {}) {
  const onStart = vi.fn((spec: AutopilotSpec): { success: boolean; error?: string } => ({ success: spec.prompt !== "" }));
  const onCancel = vi.fn();
  render(
    <AutopilotStartDialog
      tabLabel="OMP 1"
      hostname="zoo-006"
      sharedCheckout
      onCancel={onCancel}
      onStart={onStart}
      {...props}
    />,
  );
  const dialog = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
  return {
    dialog,
    onStart,
    onCancel,
    template: () => within(dialog).getByLabelText("Template") as HTMLSelectElement,
    prompt: () => within(dialog).getByLabelText("Prompt") as HTMLTextAreaElement,
    note: () => within(dialog).getByTestId("autopilot-template-note").textContent ?? "",
    toolCall: () => within(dialog).getByRole("checkbox", { name: /End when an iteration makes no tool call/ }) as HTMLInputElement,
    choose(value: string) {
      fireEvent.change(within(dialog).getByLabelText("Template"), { target: { value } });
    },
    type(value: string) {
      fireEvent.change(within(dialog).getByLabelText("Prompt"), { target: { value } });
    },
    start() {
      fireEvent.click(within(dialog).getByRole("button", { name: "Start autopilot" }));
    },
  };
}

describe("a tab with no earlier autopilot", () => {
  it("opens on the default template, with its whole text in the prompt field", () => {
    const view = open();

    expect(view.template().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.id);
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
    // Every template by name, then the entry for the person's own text.
    expect(Array.from(view.template().options).map((option) => option.textContent)).toEqual([
      "APAS worker",
      "Implement from a design document",
      "Work through TODO.md",
      "Keep the tests green",
      "My own text",
    ]);
    // What the template does, and how its run ends with the option as it is.
    expect(view.note()).toContain(DEFAULT_AUTOPILOT_TEMPLATE.description);
    expect(view.toolCall().checked).toBe(true);
    expect(view.note()).toContain(autopilotTemplateEndingNote(DEFAULT_AUTOPILOT_TEMPLATE, true));
    expect(view.note()).not.toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
    expect(view.template().getAttribute("aria-describedby"))
      .toBe(within(view.dialog).getByTestId("autopilot-template-note").id);
    // Opening it sends nothing.
    expect(view.onStart).not.toHaveBeenCalled();
  });

  it("starts the default template unchanged, with no limit, in one request", () => {
    const view = open();
    view.start();
    expect(view.onStart.mock.calls).toEqual([[{
      prompt: DEFAULT_AUTOPILOT_TEMPLATE.prompt,
      repeat: { kind: "after_each_turn" },
      iteration_limit: { kind: "none" },
      time_limit: { kind: "none" },
      end_without_tool_call: true,
    }]]);
  });

  it("replaces the prompt when another template is chosen, and nothing else", () => {
    const view = open();
    fireEvent.click(within(view.dialog).getByRole("radio", { name: "Every" }));
    fireEvent.click(within(view.dialog).getAllByRole("radio", { name: "At most" })[0]);
    fireEvent.change(within(view.dialog).getByLabelText("Iteration limit"), { target: { value: "20" } });
    fireEvent.click(view.toolCall());
    expect(view.toolCall().checked).toBe(true);

    view.choose(WATCH.id);

    expect(view.template().value).toBe(WATCH.id);
    expect(view.prompt().value).toBe(WATCH.prompt);
    expect(view.note()).toContain(WATCH.description);
    expect(view.toolCall().checked).toBe(true);
    view.start();
    expect(view.onStart.mock.calls).toEqual([[{
      prompt: WATCH.prompt,
      repeat: { kind: "every", seconds: 900 },
      iteration_limit: { kind: "at_most", value: 20 },
      time_limit: { kind: "none" },
      end_without_tool_call: true,
    }]]);
  });

  it("says what a template's ending needs instead of ticking the option for it", () => {
    const view = open();
    // A timed repeat proposes the option off, as before templates.
    fireEvent.click(within(view.dialog).getByRole("radio", { name: "Every" }));
    expect(view.toolCall().checked).toBe(false);
    expect(view.note()).toContain(autopilotTemplateEndingNote(DEFAULT_AUTOPILOT_TEMPLATE, false));

    view.choose(TODO.id);
    expect(view.toolCall().checked).toBe(false);
    expect(view.note()).toContain(autopilotTemplateEndingNote(TODO, false));

    fireEvent.click(view.toolCall());
    expect(view.note()).toContain(autopilotTemplateEndingNote(TODO, true));

    view.choose(WATCH.id);
    expect(view.toolCall().checked).toBe(true);
    expect(view.note()).toContain("has no end of its own");
  });

  it("shows an edited template as the person's own text", () => {
    const view = open();
    const edited = `${DEFAULT_AUTOPILOT_TEMPLATE.prompt}\nThe design document is docs/architecture.md.`;
    view.type(edited);

    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    expect(view.note()).toBe(AUTOPILOT_OWN_TEXT_DESCRIPTION);
    view.start();
    expect(view.onStart.mock.calls[0][0].prompt).toBe(edited);
  });

  it("keeps what the person wrote when a template is chosen, and gives it back", () => {
    const view = open();
    view.type("Fix the next failing test.");
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);

    view.choose(TODO.id);
    expect(view.prompt().value).toBe(TODO.prompt);
    // Said in the dialog, not done silently.
    expect(view.note()).toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
    view.choose(WATCH.id);
    expect(view.note()).toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);

    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe("Fix the next failing test.");
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    expect(view.note()).toBe(AUTOPILOT_OWN_TEXT_DESCRIPTION);
    expect(view.onStart).not.toHaveBeenCalled();
  });

  it("gives an empty field for own text when nothing was written yet, and refuses to start it", () => {
    const view = open();
    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe("");
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);

    view.start();
    expect(within(view.dialog).getByRole("alert").textContent).toBe("Autopilot needs a prompt.");
    expect(view.onStart).not.toHaveBeenCalled();
    // Choosing a template clears the refusal with the reason for it.
    view.choose(DEFAULT_AUTOPILOT_TEMPLATE.id);
    expect(within(view.dialog).queryByRole("alert")).toBeNull();
    expect(view.note()).not.toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
  });

  it("shows a template again when its exact text is back in the field", () => {
    const view = open();
    view.type(`${TODO.prompt}!`);
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    view.type(TODO.prompt);
    expect(view.template().value).toBe(TODO.id);
    // Nothing of the person's is set aside: the field holds what they wrote.
    expect(view.note()).not.toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
  });
});

describe("an edit to a template that is taken back", () => {
  it("leaves an empty field for own text, not the template again", () => {
    const view = open();
    // A slip at the end of the default template, deleted again.
    view.type(`${DEFAULT_AUTOPILOT_TEMPLATE.prompt}x`);
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    view.type(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
    expect(view.template().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.id);

    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe("");
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    expect(view.note()).toBe(AUTOPILOT_OWN_TEXT_DESCRIPTION);
  });

  it("does the same when the text typed back is another template's", () => {
    const view = open();
    view.type(`${TODO.prompt}!`);
    view.type(TODO.prompt);
    expect(view.template().value).toBe(TODO.id);

    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe("");
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    // And a template can be chosen again from there.
    view.choose(WATCH.id);
    expect(view.prompt().value).toBe(WATCH.prompt);
    expect(view.note()).not.toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
  });

  it("still gives back text the person wrote that is not a template's", () => {
    const view = open();
    view.type(`${TODO.prompt}\nAlso update CHANGELOG.md.`);
    view.choose(WATCH.id);
    expect(view.note()).toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe(`${TODO.prompt}\nAlso update CHANGELOG.md.`);
  });
});

describe("a tab whose last autopilot ended", () => {
  it("opens on that autopilot's prompt as the person's own text, and keeps it through a template", () => {
    const view = open({ last: LAST });
    expect(view.prompt().value).toBe(LAST.prompt);
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    expect(view.note()).toBe(AUTOPILOT_OWN_TEXT_DESCRIPTION);

    view.choose(DEFAULT_AUTOPILOT_TEMPLATE.id);
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
    expect(view.note()).toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
    // The last autopilot's timing, limits and option are not the template's to change.
    expect(view.toolCall().checked).toBe(false);
    expect(view.note()).toContain(autopilotTemplateEndingNote(DEFAULT_AUTOPILOT_TEMPLATE, false));
    expect((within(view.dialog).getByRole("radio", { name: "Every" }) as HTMLInputElement).checked).toBe(true);

    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe(LAST.prompt);
    view.start();
    expect(view.onStart.mock.calls).toEqual([[LAST]]);
  });

  it("shows the template when the last prompt was a template unchanged", () => {
    const view = open({ last: { ...LAST, prompt: TODO.prompt } });
    expect(view.template().value).toBe(TODO.id);
    expect(view.prompt().value).toBe(TODO.prompt);
    expect(view.note()).not.toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
  });
});

describe("the phone sheet", () => {
  it("has a 16px Template control that keeps its height, above a prompt field that keeps its own", () => {
    const view = open({ variant: "sheet" });
    // iOS zooms the page on a focused field with smaller text, and both
    // fields sit in a scrolling column where an unpinned child can collapse.
    expect(view.template().className.split(" ")).toEqual(expect.arrayContaining(["text-[16px]", "w-full"]));
    expect(view.template().parentElement?.className.split(" ")).toEqual(expect.arrayContaining(["flex-col", "shrink-0"]));
    expect(view.prompt().className.split(" ")).toEqual(expect.arrayContaining(["text-[16px]", "shrink-0"]));
    expect(view.prompt().rows).toBe(5);
    expect(view.template().compareDocumentPosition(view.prompt()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(view.template().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.id);
    expect(view.template().options).toHaveLength(AUTOPILOT_TEMPLATES.length + 1);
    // The keyboard does not open over the statement.
    expect(document.activeElement).not.toBe(view.prompt());
  });
});

// The dialog is taller than a laptop window. Its two buttons used to be the
// last children of one scrolling column, below the fold of a 1440x900 window.
describe.each(["dialog", "sheet"] as const)("the footer of the %s", (variant) => {
  it("stays outside the scrolling body, with Cancel, Start autopilot and a refusal", () => {
    const view = open({ variant });
    const body = within(view.dialog).getByTestId("autopilot-start-body");
    const footer = within(view.dialog).getByTestId("autopilot-start-footer");

    // Only the body scrolls; the dialog and the footer do not.
    expect(view.dialog.className.split(" ")).toEqual(expect.arrayContaining(["flex", "flex-col", "overflow-hidden"]));
    expect(view.dialog.className).not.toContain("overflow-y-auto");
    expect(body.className.split(" ")).toEqual(expect.arrayContaining(["flex-col", "min-h-0", "overflow-y-auto"]));
    expect(footer.className.split(" ")).toContain("shrink-0");
    expect(footer.className).not.toContain("overflow");
    expect(Array.from(view.dialog.children)).toEqual([body, footer]);

    // Every field is in the body, and the prompt keeps its height there.
    for (const field of [view.template(), view.prompt(), view.toolCall()]) expect(body.contains(field)).toBe(true);
    expect(view.prompt().className.split(" ")).toContain("shrink-0");
    expect(within(body).getByTestId("autopilot-limits-statement")).toBeTruthy();

    // The buttons are in the footer, and so is the reason a start was refused.
    expect(within(footer).getByRole("button", { name: "Cancel" })).toBeTruthy();
    expect(within(footer).getByRole("button", { name: "Start autopilot" })).toBeTruthy();
    view.type("");
    view.start();
    expect(within(footer).getByRole("alert").textContent).not.toBe("");
    expect(within(body).queryByRole("alert")).toBeNull();
  });
});
