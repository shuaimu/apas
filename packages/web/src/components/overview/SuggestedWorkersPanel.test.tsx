import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SuggestedWorkersPanel } from "./SuggestedWorkersPanel";
import { useStore, type SuggestedWorker } from "@/lib/store";

const SESSION_ID = "session-suggestions-panel";
const initialStore = useStore.getInitialState();

function suggestion(overrides: Partial<SuggestedWorker> = {}): SuggestedWorker {
  return {
    id: "SUG-001",
    label: "Frontend Worker",
    role: "developer",
    goal: "Build the dashboard",
    backstory: "React specialist",
    needs_worktree: true,
    ...overrides,
  };
}

function seedSuggestedWorkers(suggestions: SuggestedWorker[] | null) {
  const fetchSuggestedWorkers = vi.fn();
  const acceptSuggestion = vi.fn();
  const dismissSuggestion = vi.fn();
  const suggestedWorkersBySession = new Map<string, SuggestedWorker[]>();
  if (suggestions !== null) {
    suggestedWorkersBySession.set(SESSION_ID, suggestions);
  }

  act(() => {
    useStore.setState({
      sessionId: SESSION_ID,
      suggestedWorkersBySession,
      fetchSuggestedWorkers,
      acceptSuggestion,
      dismissSuggestion,
    });
  });

  return { fetchSuggestedWorkers, acceptSuggestion, dismissSuggestion };
}

describe("SuggestedWorkersPanel", () => {
  afterEach(() => {
    vi.restoreAllMocks();
    act(() => {
      useStore.setState(initialStore, true);
    });
  });

  it("fetches suggestions and shows loading before state arrives", () => {
    const { fetchSuggestedWorkers } = seedSuggestedWorkers(null);

    render(<SuggestedWorkersPanel />);

    expect(fetchSuggestedWorkers).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Loading suggestions…")).toBeTruthy();
  });

  it("renders the empty state when the session has no suggestions", () => {
    const { fetchSuggestedWorkers } = seedSuggestedWorkers([]);

    render(<SuggestedWorkersPanel />);

    expect(fetchSuggestedWorkers).toHaveBeenCalledTimes(1);
    expect(screen.getByText(/No suggestions yet/)).toBeTruthy();
    expect(screen.getByText("Suggest workers")).toBeTruthy();
  });

  it("renders suggestion fields and wires Accept and Dismiss actions", () => {
    const worker = suggestion();
    const { acceptSuggestion, dismissSuggestion } = seedSuggestedWorkers([worker]);

    render(<SuggestedWorkersPanel />);

    expect(screen.getByText("Frontend Worker")).toBeTruthy();
    expect(screen.getByText("SUG-001")).toBeTruthy();
    expect(screen.getByText("developer")).toBeTruthy();
    expect(screen.getByText("Build the dashboard")).toBeTruthy();
    expect(screen.getByText("React specialist")).toBeTruthy();
    expect(screen.getByText("Will get an isolated worktree")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /accept/i }));
    fireEvent.click(screen.getByRole("button", { name: /dismiss/i }));

    expect(acceptSuggestion).toHaveBeenCalledWith(worker);
    expect(dismissSuggestion).toHaveBeenCalledWith("SUG-001");
  });
});
