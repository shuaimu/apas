import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import {
  DELETION_STILL_IN_PROGRESS,
  PROJECT_API_URL,
  requestProjectDeletion,
  waitForProjectDeletion,
} from "./projectDeletion";

let fetchMock: Mock;

function response(status: number, body: unknown = {}) {
  return { status, ok: status >= 200 && status < 300, json: async () => body };
}

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal("fetch", fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("requestProjectDeletion", () => {
  it("posts once with the project id as the confirmation and resolves on 202", async () => {
    fetchMock.mockResolvedValueOnce(response(202));
    await expect(requestProjectDeletion("project-1", "token-1")).resolves.toBeUndefined();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock).toHaveBeenCalledWith(`${PROJECT_API_URL}/projects/project-1/delete`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: "Bearer token-1" },
      body: JSON.stringify({ confirmation: "project-1" }),
    });
  });

  it("rejects with the server's message, and with a fallback when it gave none", async () => {
    fetchMock.mockResolvedValueOnce(response(409, { error: "Deletion already started" }));
    await expect(requestProjectDeletion("project-1", "token-1")).rejects.toThrow("Deletion already started");
    fetchMock.mockResolvedValueOnce({ status: 500, ok: false, json: async () => { throw new Error("not json"); } });
    await expect(requestProjectDeletion("project-1", "token-1")).rejects.toThrow("Failed to delete project");
    // A 200 is not an acceptance.
    fetchMock.mockResolvedValueOnce(response(200));
    await expect(requestProjectDeletion("project-1", "token-1")).rejects.toThrow("Failed to delete project");
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });
});

describe("waitForProjectDeletion", () => {
  it("asks until the project is gone", async () => {
    fetchMock
      .mockResolvedValueOnce(response(200))
      .mockResolvedValueOnce(response(409))
      .mockResolvedValueOnce(response(404));
    await expect(waitForProjectDeletion("project-1", "token-1", { intervalMs: 1 })).resolves.toBeUndefined();
    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(fetchMock).toHaveBeenLastCalledWith(`${PROJECT_API_URL}/projects/project-1/deletion`, {
      headers: { Authorization: "Bearer token-1" },
    });
  });

  it("rejects with the server's message on any other answer", async () => {
    fetchMock.mockResolvedValueOnce(response(403, { message: "Access denied" }));
    await expect(waitForProjectDeletion("project-1", "token-1", { intervalMs: 1 })).rejects.toThrow("Access denied");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("stops asking after the last attempt and says the deletion goes on", async () => {
    fetchMock.mockResolvedValue(response(200));
    await expect(waitForProjectDeletion("project-1", "token-1", { intervalMs: 1, attempts: 2 }))
      .rejects.toThrow(DELETION_STILL_IN_PROGRESS);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
