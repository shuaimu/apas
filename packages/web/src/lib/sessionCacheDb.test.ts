import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionCacheEntry } from "./store";

const idb = vi.hoisted(() => {
  const store = { db: "apas-cache", store: "session-snapshots" };
  return {
    clear: vi.fn(),
    createStore: vi.fn(() => store),
    del: vi.fn(),
    entries: vi.fn(),
    get: vi.fn(),
    set: vi.fn(),
    store,
  };
});

vi.mock("idb-keyval", () => ({
  clear: idb.clear,
  createStore: idb.createStore,
  del: idb.del,
  entries: idb.entries,
  get: idb.get,
  set: idb.set,
}));

function makeEntry(overrides: Partial<SessionCacheEntry> = {}): SessionCacheEntry {
  return {
    answeredQuestions: new Map(),
    cachedAt: 123,
    hasMoreMessages: false,
    isDualPane: false,
    messages: [],
    paneConfigs: [],
    paneHasMore: {},
    paneMessages: {},
    paneModes: {},
    ...overrides,
  };
}

async function importDb() {
  return import("./sessionCacheDb");
}

function installIndexedDb() {
  const fakeIndexedDb = {} as IDBFactory;
  Object.defineProperty(globalThis, "indexedDB", {
    configurable: true,
    value: fakeIndexedDb,
  });
  Object.defineProperty(window, "indexedDB", {
    configurable: true,
    value: fakeIndexedDb,
  });
}

function removeIndexedDb() {
  Object.defineProperty(globalThis, "indexedDB", {
    configurable: true,
    value: undefined,
  });
  Object.defineProperty(window, "indexedDB", {
    configurable: true,
    value: undefined,
  });
}

async function flushPromises() {
  await Promise.resolve();
  await Promise.resolve();
}

describe("sessionCacheDb", () => {
  let warnSpy: ReturnType<typeof vi.spyOn>;
  let originalWindow: Window & typeof globalThis;

  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    idb.createStore.mockReturnValue(idb.store);
    idb.entries.mockResolvedValue([]);
    idb.get.mockResolvedValue(undefined);
    idb.set.mockResolvedValue(undefined);
    idb.del.mockResolvedValue(undefined);
    idb.clear.mockResolvedValue(undefined);
    originalWindow = window;
    installIndexedDb();
    warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
  });

  afterEach(() => {
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: originalWindow,
      writable: true,
    });
    removeIndexedDb();
    warnSpy.mockRestore();
  });

  it("loads only string-keyed object snapshots from the custom APAS store", async () => {
    const validEntry = makeEntry({ cachedAt: 456 });
    idb.entries.mockResolvedValue([
      ["session-a", validEntry],
      [17, makeEntry()],
      ["missing-value", null],
      ["string-value", "not-a-snapshot"],
    ]);

    const { loadAllSnapshots } = await importDb();
    const snapshots = await loadAllSnapshots();

    expect(idb.createStore).toHaveBeenCalledWith("apas-cache", "session-snapshots");
    expect(idb.entries).toHaveBeenCalledWith(idb.store);
    expect(snapshots).toEqual(new Map([["session-a", validEntry]]));
  });

  it("uses the custom APAS store for single snapshot and clear operations", async () => {
    const entry = makeEntry({ lastCreatedAt: "2026-06-17T18:00:00Z" });
    idb.get.mockResolvedValue(entry);

    const {
      clearAllSnapshots,
      deleteSnapshot,
      loadSnapshot,
      saveSnapshot,
    } = await importDb();

    await expect(loadSnapshot("session-a")).resolves.toBe(entry);
    saveSnapshot("session-a", entry);
    deleteSnapshot("session-a");
    await clearAllSnapshots();

    expect(idb.get).toHaveBeenCalledWith("session-a", idb.store);
    expect(idb.set).toHaveBeenCalledWith("session-a", entry, idb.store);
    expect(idb.del).toHaveBeenCalledWith("session-a", idb.store);
    expect(idb.clear).toHaveBeenCalledWith(idb.store);
    expect(idb.createStore).toHaveBeenCalledTimes(1);
  });

  it("falls back safely when no browser window is available", async () => {
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: undefined,
      writable: true,
    });

    const {
      clearAllSnapshots,
      deleteSnapshot,
      loadAllSnapshots,
      loadSnapshot,
      saveSnapshot,
    } = await importDb();

    await expect(loadAllSnapshots()).resolves.toEqual(new Map());
    await expect(loadSnapshot("session-a")).resolves.toBeUndefined();
    expect(() => saveSnapshot("session-a", makeEntry())).not.toThrow();
    expect(() => deleteSnapshot("session-a")).not.toThrow();
    await expect(clearAllSnapshots()).resolves.toBeUndefined();
    expect(idb.createStore).not.toHaveBeenCalled();
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it("falls back safely when IndexedDB is unavailable", async () => {
    removeIndexedDb();

    const {
      clearAllSnapshots,
      deleteSnapshot,
      loadAllSnapshots,
      loadSnapshot,
      saveSnapshot,
    } = await importDb();

    await expect(loadAllSnapshots()).resolves.toEqual(new Map());
    await expect(loadSnapshot("session-a")).resolves.toBeUndefined();
    expect(() => saveSnapshot("session-a", makeEntry())).not.toThrow();
    expect(() => deleteSnapshot("session-a")).not.toThrow();
    await expect(clearAllSnapshots()).resolves.toBeUndefined();
    expect(idb.createStore).not.toHaveBeenCalled();
    expect(warnSpy).not.toHaveBeenCalled();
  });

  it("warns and returns safe fallbacks when idb-keyval operations reject", async () => {
    const loadAllError = new Error("entries failed");
    const loadOneError = new Error("get failed");
    const saveError = new Error("set failed");
    const deleteError = new Error("del failed");
    const clearError = new Error("clear failed");
    idb.entries.mockRejectedValueOnce(loadAllError);
    idb.get.mockRejectedValueOnce(loadOneError);
    idb.set.mockRejectedValueOnce(saveError);
    idb.del.mockRejectedValueOnce(deleteError);
    idb.clear.mockRejectedValueOnce(clearError);

    const {
      clearAllSnapshots,
      deleteSnapshot,
      loadAllSnapshots,
      loadSnapshot,
      saveSnapshot,
    } = await importDb();

    await expect(loadAllSnapshots()).resolves.toEqual(new Map());
    await expect(loadSnapshot("session-a")).resolves.toBeUndefined();
    saveSnapshot("session-a", makeEntry());
    deleteSnapshot("session-a");
    await clearAllSnapshots();
    await flushPromises();

    expect(warnSpy).toHaveBeenCalledWith(
      "[sessionCacheDb] loadAllSnapshots failed",
      loadAllError,
    );
    expect(warnSpy).toHaveBeenCalledWith(
      "[sessionCacheDb] loadSnapshot failed",
      loadOneError,
    );
    expect(warnSpy).toHaveBeenCalledWith(
      "[sessionCacheDb] saveSnapshot failed",
      saveError,
    );
    expect(warnSpy).toHaveBeenCalledWith(
      "[sessionCacheDb] deleteSnapshot failed",
      deleteError,
    );
    expect(warnSpy).toHaveBeenCalledWith(
      "[sessionCacheDb] clearAllSnapshots failed",
      clearError,
    );
  });
});
