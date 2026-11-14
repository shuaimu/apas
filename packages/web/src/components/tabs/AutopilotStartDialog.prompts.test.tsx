import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AutopilotPromptChange, AutopilotSpec, SavedAutopilotPrompt } from "@apas/protocol";
import { useStore } from "@/lib/store";
import { DEFAULT_AUTOPILOT_TEMPLATE, autopilotTemplateById, AUTOPILOT_OWN_TEXT, AUTOPILOT_OWN_TEXT_KEPT_TEXT } from "@/lib/autopilotTemplates";
import {
  PROJECT_AUTOPILOT_PROMPTS_CAPABILITY,
  type AutopilotPromptsRequest,
  type ProjectAutopilotPrompts,
} from "@/lib/autopilotPrompts";
import { AutopilotStartDialog } from "./AutopilotStartDialog";

/**
 * The start dialog with a project's saved prompts: how they are listed and
 * offered, and how the dialog saves, replaces, deletes and sets the default.
 * The store's side (one frame per change, receipts, never resent) is covered
 * in `store.autopilotPrompts.test.ts`; here the store action is a stand-in
 * that records what the dialog asks for.
 */

const SID = "20000000-0000-4000-8000-000000000001";

const NIGHTLY: SavedAutopilotPrompt = {
  name: "Nightly review",
  prompt: "Review the open pull requests and fix what the checks report.",
  updated_at: "2026-11-14T09:00:00Z",
};
const RELEASE: SavedAutopilotPrompt = {
  name: "Release check",
  prompt: "Run the release checklist in docs/release.md.\nStop at the first failure.",
  updated_at: "2026-11-14T09:05:00Z",
};

const LAST: AutopilotSpec = {
  prompt: "Check the build and fix what fails.",
  repeat: { kind: "after_each_turn" },
  iteration_limit: { kind: "none" },
  time_limit: { kind: "none" },
  end_without_tool_call: true,
};

const initialState = useStore.getState();
let sent: AutopilotPromptChange[] = [];
let nextId = 0;

function list(value: Partial<ProjectAutopilotPrompts> = {}): ProjectAutopilotPrompts {
  return { prompts: [NIGHTLY, RELEASE], default: NIGHTLY.name, unreadable: false, received: true, ...value };
}

function setUp({ capable = true, prompts = list() }: { capable?: boolean; prompts?: ProjectAutopilotPrompts | null } = {}) {
  useStore.setState({
    negotiatedCapabilities: new Set(capable ? [PROJECT_AUTOPILOT_PROMPTS_CAPABILITY] : []),
    projectAutopilotPrompts: prompts ? { [SID]: prompts } : {},
    autopilotPromptRequests: {},
  });
}

/** Settle the dialog's last change as the store would on its receipt. */
function settle(status: AutopilotPromptsRequest["status"], extra: Partial<AutopilotPromptsRequest> = {}) {
  act(() => {
    useStore.setState((state) => {
      const id = `prompts-${nextId}`;
      return { autopilotPromptRequests: { ...state.autopilotPromptRequests, [id]: { ...state.autopilotPromptRequests[id], status, ...extra } } };
    });
  });
}

beforeEach(() => {
  useStore.setState(initialState, true);
  sent = [];
  nextId = 0;
  useStore.setState({
    updateAutopilotPrompts: (sessionId: string, change: AutopilotPromptChange) => {
      sent.push(change);
      nextId += 1;
      const requestId = `prompts-${nextId}`;
      useStore.setState((state) => ({
        autopilotPromptRequests: {
          ...state.autopilotPromptRequests,
          [requestId]: { requestId, sessionId, change, status: "pending", sentAt: 0 },
        },
      }));
      return { success: true, requestId };
    },
  });
  setUp();
});

afterEach(() => {
  useStore.setState(initialState, true);
});

function open(props: Partial<Parameters<typeof AutopilotStartDialog>[0]> = {}) {
  const onStart = vi.fn((): { success: boolean; error?: string } => ({ success: true }));
  const onCancel = vi.fn();
  render(
    <AutopilotStartDialog
      sessionId={SID}
      tabLabel="OMP 1"
      hostname="zoo-006"
      sharedCheckout
      onCancel={onCancel}
      onStart={onStart}
      {...props}
    />,
  );
  const dialog = screen.getByRole("dialog", { name: "Start autopilot — OMP 1" });
  const button = (name: string | RegExp) => within(dialog).getByRole("button", { name });
  return {
    dialog,
    onStart,
    onCancel,
    button,
    template: () => within(dialog).getByLabelText("Template") as HTMLSelectElement,
    prompt: () => within(dialog).getByLabelText("Prompt") as HTMLTextAreaElement,
    note: () => within(dialog).getByTestId("autopilot-template-note").textContent ?? "",
    outcome: () => within(dialog).queryByTestId("autopilot-prompts-outcome")?.textContent ?? null,
    nameField: () => within(dialog).getByLabelText("Name") as HTMLInputElement,
    choose(value: string) {
      fireEvent.change(within(dialog).getByLabelText("Template"), { target: { value } });
    },
    chooseSaved(name: string) {
      const option = Array.from(this.template().options).find((candidate) => candidate.textContent?.replace(/ \(default\)$/, "") === name)!;
      this.choose(option.value);
    },
    type(value: string) {
      fireEvent.change(within(dialog).getByLabelText("Prompt"), { target: { value } });
    },
    typeName(value: string) {
      fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value } });
    },
  };
}

function optionTexts(select: HTMLSelectElement): string[] {
  return Array.from(select.options).map((option) => option.textContent ?? "");
}

function groups(select: HTMLSelectElement): Array<{ label: string; options: string[] }> {
  return Array.from(select.querySelectorAll("optgroup")).map((group) => ({
    label: group.label,
    options: Array.from(group.querySelectorAll("option")).map((option) => option.textContent ?? ""),
  }));
}

describe("the Template control", () => {
  it("lists the project's prompts first, marks the default, then the built-in ones, then own text", () => {
    const view = open();
    expect(groups(view.template())).toEqual([
      { label: "This project", options: ["Nightly review (default)", "Release check"] },
      {
        label: "Built in",
        options: ["APAS worker", "Implement from a design document", "Work through TODO.md", "Keep the tests green"],
      },
    ]);
    expect(optionTexts(view.template()).at(-1)).toBe("My own text");
  });

  it("has no project group while the project has no saved prompts, and still offers saving", () => {
    setUp({ prompts: list({ prompts: [], default: undefined }) });
    const view = open();
    expect(groups(view.template()).map((group) => group.label)).toEqual(["Built in"]);
    expect(view.button("Save to project…")).toBeTruthy();
  });

  it("shows nothing of saved prompts when the server did not negotiate them", () => {
    setUp({ capable: false });
    const view = open();
    expect(groups(view.template())).toEqual([]);
    expect(optionTexts(view.template())).toEqual([
      "APAS worker", "Implement from a design document", "Work through TODO.md", "Keep the tests green", "My own text",
    ]);
    expect(within(view.dialog).queryByTestId("autopilot-saved-prompts")).toBeNull();
    expect(within(view.dialog).queryByRole("button", { name: "Save to project…" })).toBeNull();
    // Nor is the project's default offered.
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
  });

  it("shows nothing of saved prompts without a session", () => {
    const view = open({ sessionId: undefined });
    expect(groups(view.template())).toEqual([]);
    expect(within(view.dialog).queryByTestId("autopilot-saved-prompts")).toBeNull();
  });

  it("treats a saved prompt like a template: choosing replaces the text, and editing makes it own text", () => {
    const view = open({ last: LAST });
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);

    view.chooseSaved("Release check");
    expect(view.prompt().value).toBe(RELEASE.prompt);
    expect(optionTexts(view.template())[view.template().selectedIndex]).toBe("Release check");
    expect(view.note()).toContain("Saved to this project.");
    expect(view.note()).not.toContain("default");
    // The last autopilot's prompt was the person's own: it is kept.
    expect(view.note()).toContain(AUTOPILOT_OWN_TEXT_KEPT_TEXT);
    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe(LAST.prompt);

    view.chooseSaved("Nightly review");
    expect(view.note()).toContain("It is the project's default.");
    view.type(`${NIGHTLY.prompt} Then stop.`);
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    // Taken back to the exact text, the control names it again.
    view.type(NIGHTLY.prompt);
    expect(optionTexts(view.template())[view.template().selectedIndex]).toBe("Nightly review (default)");
    // Choosing an entry sent nothing.
    expect(sent).toEqual([]);
    expect(view.onStart).not.toHaveBeenCalled();
  });

  it("names the saved prompt chosen when it holds a built-in template's text", () => {
    const copy = { ...RELEASE, name: "Copy of the worker", prompt: DEFAULT_AUTOPILOT_TEMPLATE.prompt };
    setUp({ prompts: list({ prompts: [copy], default: undefined }) });
    const view = open({ last: LAST });
    view.choose(DEFAULT_AUTOPILOT_TEMPLATE.id);
    expect(view.template().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.id);
    view.chooseSaved("Copy of the worker");
    expect(optionTexts(view.template())[view.template().selectedIndex]).toBe("Copy of the worker");
  });
});

describe("the starting text", () => {
  it("is the last ended autopilot's prompt when there is one", () => {
    const view = open({ last: LAST });
    expect(view.prompt().value).toBe(LAST.prompt);
  });

  it("is the project's default when the tab had no autopilot, and the control names it", () => {
    const view = open();
    expect(view.prompt().value).toBe(NIGHTLY.prompt);
    expect(optionTexts(view.template())[view.template().selectedIndex]).toBe("Nightly review (default)");
  });

  it("is APAS worker when the project has no default", () => {
    setUp({ prompts: list({ default: undefined }) });
    const view = open();
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
    expect(view.template().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.id);
  });

  it("becomes the project's default when the list arrives after the dialog opened, unless the person has acted", () => {
    setUp({ prompts: null });
    const view = open();
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
    act(() => useStore.setState({ projectAutopilotPrompts: { [SID]: list() } }));
    expect(view.prompt().value).toBe(NIGHTLY.prompt);
  });

  it("stays as the person left it when the list arrives later", () => {
    setUp({ prompts: null });
    const view = open();
    view.choose("todo");
    view.choose(DEFAULT_AUTOPILOT_TEMPLATE.id);
    act(() => useStore.setState({ projectAutopilotPrompts: { [SID]: list() } }));
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
  });
});

describe("Save to project", () => {
  it("saves the field's text under a new name, once, and leaves the dialog and the tab alone", () => {
    const view = open({ last: LAST });
    fireEvent.click(view.button("Save to project…"));
    expect(view.nameField().value).toBe("");
    view.typeName("  Build fixer ");
    expect(within(view.dialog).queryByText(/This replaces/)).toBeNull();
    fireEvent.click(view.button("Save"));

    expect(sent).toEqual([{ kind: "save", name: "Build fixer", prompt: LAST.prompt, replace: false }]);
    expect(view.outcome()).toBe("Saving…");
    // Pending: nothing more can be asked for until it is answered.
    expect((view.button("Save") as HTMLButtonElement).disabled).toBe(true);

    settle("saved");
    expect(view.outcome()).toBe("Saved to the project as \"Build fixer\".");
    // The name field closes once it is saved; the dialog stays open.
    expect(within(view.dialog).queryByLabelText("Name")).toBeNull();
    expect(view.onStart).not.toHaveBeenCalled();
    expect(view.onCancel).not.toHaveBeenCalled();
    expect(sent).toHaveLength(1);

    // Once the list has it, the control names it.
    act(() => useStore.setState({
      projectAutopilotPrompts: { [SID]: list({ prompts: [NIGHTLY, RELEASE, { name: "Build fixer", prompt: LAST.prompt, updated_at: "" }] }) },
    }));
    expect(optionTexts(view.template())[view.template().selectedIndex]).toBe("Build fixer");
  });

  it("replaces a saved prompt only after saying so", () => {
    const view = open({ last: LAST });
    view.chooseSaved("Release check");
    view.type(`${RELEASE.prompt}\nThen tag the release.`);
    fireEvent.click(view.button("Save to project…"));
    // Pre-filled with the saved prompt the text came from.
    expect(view.nameField().value).toBe("Release check");
    expect(within(view.dialog).getByText("This replaces the saved prompt \"Release check\".")).toBeTruthy();
    fireEvent.click(view.button("Replace"));
    expect(sent).toEqual([{
      kind: "save",
      name: "Release check",
      prompt: `${RELEASE.prompt}\nThen tag the release.`,
      replace: true,
    }]);
  });

  it("says when a name differing only in case will replace one", () => {
    const view = open();
    fireEvent.click(view.button("Save to project…"));
    view.typeName("release CHECK");
    expect(within(view.dialog).getByText("This replaces the saved prompt \"Release check\".")).toBeTruthy();
    view.typeName("Release checks");
    expect(within(view.dialog).queryByText(/This replaces/)).toBeNull();
  });

  it("shows a refusal inline, and sends nothing for a name it can refuse itself", () => {
    const view = open();
    fireEvent.click(view.button("Save to project…"));
    view.typeName("apas worker");
    fireEvent.click(view.button("Save"));
    expect(sent).toEqual([]);
    expect(view.outcome()).toBe("\"APAS worker\" is the name of a built-in template. Choose another name.");

    view.typeName("Weekly");
    fireEvent.click(view.button("Save"));
    settle("refused", { reason: "zoo-006 runs an older APAS that cannot save prompts. Reboot zoo-006 to update it." });
    expect(view.outcome()).toBe("zoo-006 runs an older APAS that cannot save prompts. Reboot zoo-006 to update it.");
    // The name field stays, to try again by hand.
    expect(view.nameField().value).toBe("Weekly");
    expect(sent).toHaveLength(1);
  });

  it("reports a change that was not confirmed, and does not send it again", () => {
    const view = open();
    fireEvent.click(view.button("Save to project…"));
    view.typeName("Weekly");
    fireEvent.click(view.button("Save"));
    settle("unconfirmed", { cause: "timeout" });
    expect(view.outcome()).toMatch(/^Not confirmed\./);
    expect(sent).toHaveLength(1);
  });

  it("starts empty after a saved prompt was left for own text, so it cannot replace that prompt", () => {
    const view = open({ last: LAST });
    view.chooseSaved("Nightly review");
    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe(LAST.prompt);
    fireEvent.click(view.button("Save to project…"));
    expect(view.nameField().value).toBe("");
    expect(within(view.dialog).queryByText(/This replaces/)).toBeNull();
    expect(within(view.dialog).queryByRole("button", { name: "Replace" })).toBeNull();
    // Enter with the empty name is refused here, and replaces nothing.
    fireEvent.keyDown(view.nameField(), { key: "Enter" });
    expect(sent.filter((change) => change.kind === "save" && change.replace)).toEqual([]);
  });

  it("keeps a pending change and its answer in view while the name field is typed in", () => {
    const view = open();
    fireEvent.click(view.button("Save to project…"));
    view.typeName("Weekly");
    fireEvent.click(view.button("Save"));
    expect(view.outcome()).toBe("Saving…");
    expect(view.nameField().readOnly).toBe(true);

    view.typeName("Weekly cleanup");
    fireEvent.keyDown(view.nameField(), { key: "Enter" });
    expect(view.outcome()).toBe("Saving…");
    expect(view.nameField().value).toBe("Weekly");
    expect((view.button("Save") as HTMLButtonElement).disabled).toBe(true);
    expect((view.button("Save to project…") as HTMLButtonElement).disabled).toBe(true);
    expect(sent).toHaveLength(1);

    settle("refused", { reason: "A saved prompt named \"Weekly\" already exists." });
    expect(view.outcome()).toBe("A saved prompt named \"Weekly\" already exists.");
    expect(view.nameField().readOnly).toBe(false);
    // Once answered, a new name clears the answer.
    view.typeName("Weekly cleanup");
    expect(view.outcome()).toBeNull();
  });

  it("can be cancelled without sending anything", () => {
    const view = open();
    fireEvent.click(view.button("Save to project…"));
    fireEvent.click(view.button("Cancel saving"));
    expect(within(view.dialog).queryByLabelText("Name")).toBeNull();
    expect(sent).toEqual([]);
    expect(view.onCancel).not.toHaveBeenCalled();
  });
});

describe("a selected saved prompt", () => {
  it("is deleted only after a confirmation", () => {
    const view = open();
    expect(within(view.dialog).queryByRole("button", { name: "Delete it" })).toBeNull();
    fireEvent.click(view.button("Delete"));
    expect(within(view.dialog).getByTestId("autopilot-delete-prompt").textContent)
      .toContain("Delete the saved prompt \"Nightly review\" from this project?");
    fireEvent.click(view.button("Keep it"));
    expect(sent).toEqual([]);

    fireEvent.click(view.button("Delete"));
    fireEvent.click(view.button("Delete it"));
    expect(sent).toEqual([{ kind: "delete", name: "Nightly review" }]);
    expect(view.outcome()).toBe("Deleting…");
    settle("saved");
    expect(view.outcome()).toBe("Deleted \"Nightly review\".");

    // Gone from the list, its text is the person's own and stays in the field.
    act(() => useStore.setState({ projectAutopilotPrompts: { [SID]: list({ prompts: [RELEASE], default: undefined }) } }));
    expect(view.template().value).toBe(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe(NIGHTLY.prompt);
    view.choose("todo");
    view.choose(AUTOPILOT_OWN_TEXT);
    expect(view.prompt().value).toBe(NIGHTLY.prompt);
    expect(view.onStart).not.toHaveBeenCalled();
  });

  it("offers Make default, or Clear default for the default", () => {
    const view = open();
    // The default is selected.
    fireEvent.click(view.button("Clear default"));
    expect(sent).toEqual([{ kind: "set_default", name: null }]);
    settle("saved");
    expect(view.outcome()).toBe("The project has no default prompt now.");

    view.chooseSaved("Release check");
    fireEvent.click(view.button("Make default"));
    expect(sent.at(-1)).toEqual({ kind: "set_default", name: "Release check" });
    settle("saved");
    expect(view.outcome()).toBe("\"Release check\" is the project's default.");
    expect(view.onStart).not.toHaveBeenCalled();
  });

  it("has neither for a built-in template or own text", () => {
    const view = open();
    view.choose(autopilotTemplateById("todo")!.id);
    expect(within(view.dialog).queryByRole("button", { name: "Delete" })).toBeNull();
    expect(within(view.dialog).queryByRole("button", { name: /default/ })).toBeNull();
    view.type("Something else.");
    expect(within(view.dialog).queryByRole("button", { name: "Delete" })).toBeNull();
  });
});

describe("an unreadable list", () => {
  it("says so, lists no project prompts and offers no save", () => {
    setUp({ prompts: { prompts: [], unreadable: true, received: true } });
    const view = open();
    expect(within(view.dialog).getByText("This project's saved prompts cannot be read.")).toBeTruthy();
    expect(within(view.dialog).queryByRole("button", { name: "Save to project…" })).toBeNull();
    expect(groups(view.template()).map((group) => group.label)).toEqual(["Built in"]);
    expect(view.prompt().value).toBe(DEFAULT_AUTOPILOT_TEMPLATE.prompt);
  });
});

describe("the phone sheet", () => {
  it("uses 16px text in the name field, so iOS does not zoom", () => {
    const view = open({ variant: "sheet" });
    fireEvent.click(view.button("Save to project…"));
    expect(view.nameField().className).toContain("text-[16px]");
    expect(view.template().className).toContain("text-[16px]");
  });
});

describe("focus", () => {
  it("moves into the save form, stays there while saving, and returns to Save to project… after", () => {
    const view = open();
    fireEvent.click(view.button("Save to project…"));
    expect(document.activeElement).toBe(view.nameField());
    view.typeName("Weekly");
    fireEvent.click(view.button("Save"));
    expect(document.activeElement).toBe(view.nameField());
    settle("saved");
    expect(within(view.dialog).queryByLabelText("Name")).toBeNull();
    expect(document.activeElement).toBe(view.button("Save to project…"));

    fireEvent.click(view.button("Save to project…"));
    fireEvent.click(view.button("Cancel saving"));
    expect(document.activeElement).toBe(view.button("Save to project…"));
  });

  it("moves into the delete confirmation, and back to Delete when it is kept", () => {
    const view = open();
    fireEvent.click(view.button("Delete"));
    expect(document.activeElement).toBe(view.button("Keep it"));
    fireEvent.click(view.button("Keep it"));
    expect(document.activeElement).toBe(view.button("Delete"));
  });

  it("goes to the Template control when Delete cannot take it back", () => {
    const view = open();
    fireEvent.click(view.button("Delete"));
    fireEvent.click(view.button("Delete it"));
    // Delete is disabled while the change is pending.
    expect(document.activeElement).toBe(view.template());
    settle("saved");
    act(() => useStore.setState({ projectAutopilotPrompts: { [SID]: list({ prompts: [RELEASE], default: undefined }) } }));
    expect(within(view.dialog).queryByRole("button", { name: "Delete" })).toBeNull();
    expect(document.activeElement).toBe(view.template());
  });

  it("is left where the person put it", () => {
    const view = open();
    fireEvent.click(view.button("Delete"));
    view.template().focus();
    view.choose("todo");
    expect(within(view.dialog).queryByTestId("autopilot-delete-prompt")).toBeNull();
    expect(document.activeElement).toBe(view.template());
  });
});
