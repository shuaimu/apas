"use client";

import { useEffect, useRef, useState } from "react";
import type { MachineInfo, MachineWithProjects } from "@/lib/store";
import {
  OverviewSection,
  overviewButton,
  overviewCard,
  overviewMuted,
  overviewPrimaryButton,
  overviewQuietDangerButton,
} from "@/components/overview/OverviewElements";
import type { ConfirmRequest } from "./ConfirmDialog";

const DEEPSEEK_API_BASE_URL = "https://api.deepseek.com/anthropic";
const NOT_CONNECTED = "Not connected to the server";
const NOT_SENT = "Not connected to the server. Nothing was sent.";

type SetDeepseekConfig = (machineId: string, apiKey?: string, clearApiKey?: boolean) => boolean;

/**
 * Each machine's DeepSeek key, by whether it has one. The key itself is never
 * shown, prefilled or kept: the page cannot read it. A new key is typed into a
 * password field that starts empty, sent once and forgotten.
 */
export function DeepseekKeyList({
  machines,
  connected,
  setMachineDeepseekConfig,
  requestConfirm,
}: {
  machines: MachineWithProjects[];
  connected: boolean;
  /** Returns whether the request was sent. */
  setMachineDeepseekConfig: SetDeepseekConfig;
  requestConfirm: (request: ConfirmRequest) => void;
}) {
  // One editor at a time across the list.
  const [editing, setEditing] = useState<string | null>(null);

  return (
    <OverviewSection
      title="DeepSeek keys"
      description={<>Claude tabs that use DeepSeek send their requests to <code className="font-mono">{DEEPSEEK_API_BASE_URL}</code> with the key set on their machine.</>}
    >
      <div className={`${overviewCard} divide-y divide-gray-100 dark:divide-gray-700/60`}>
        {machines.map(({ machine }) => (
          <DeepseekKeyRow
            key={machine.machineId}
            machine={machine}
            connected={connected}
            open={editing === machine.machineId}
            onOpen={() => setEditing(machine.machineId)}
            onClose={() => setEditing((current) => current === machine.machineId ? null : current)}
            setMachineDeepseekConfig={setMachineDeepseekConfig}
            requestConfirm={requestConfirm}
          />
        ))}
      </div>
    </OverviewSection>
  );
}

function DeepseekKeyRow({
  machine,
  connected,
  open,
  onOpen,
  onClose,
  setMachineDeepseekConfig,
  requestConfirm,
}: {
  machine: MachineInfo;
  connected: boolean;
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
  setMachineDeepseekConfig: SetDeepseekConfig;
  requestConfirm: (request: ConfirmRequest) => void;
}) {
  const [draft, setDraft] = useState("");
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const opener = useRef<HTMLButtonElement | null>(null);
  const field = useRef<HTMLInputElement | null>(null);
  const wasOpen = useRef(open);
  // Set only when this row's own Cancel, Escape or Save closes it: opening
  // another row's editor closes this one too, and must keep its focus.
  const returnFocus = useRef(false);
  const hostname = machine.hostname;
  const configured = machine.deepseekBackend?.apiKeyConfigured === true;
  const trimmed = draft.trim();

  // The field takes focus when the editor opens, and the button that opened
  // it gets focus back when it closes, so a keyboard user is not dropped at
  // the top of the page.
  useEffect(() => {
    if (open && !wasOpen.current) field.current?.focus();
    if (!open && wasOpen.current && returnFocus.current) opener.current?.focus();
    returnFocus.current = false;
    wasOpen.current = open;
  }, [open]);

  useEffect(() => {
    if (!result?.ok) return;
    const timer = window.setTimeout(() => setResult(null), 4000);
    return () => window.clearTimeout(timer);
  }, [result]);

  const close = () => { setDraft(""); returnFocus.current = true; onClose(); };

  return (
    <div className="px-4 py-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <div className="min-w-0">
          <div className="break-all text-sm font-medium text-gray-900 dark:text-gray-100">{hostname}</div>
          <div className={overviewMuted}>{configured ? "API key configured" : "API key not configured"}</div>
        </div>
        <div className="ml-auto flex flex-wrap items-center justify-end gap-1">
          {result && (
            <span
              role={result.ok ? "status" : "alert"}
              className={result.ok ? overviewMuted : "text-xs text-red-700 dark:text-red-300"}
            >
              {result.text}
            </span>
          )}
          {!open && (
            <button
              ref={opener}
              type="button"
              aria-label={`${configured ? "Replace" : "Set"} DeepSeek API key for ${hostname}`}
              title={connected ? undefined : NOT_CONNECTED}
              disabled={!connected}
              onClick={() => { setDraft(""); setResult(null); onOpen(); }}
              className={overviewButton}
            >
              {configured ? "Replace key" : "Set key"}
            </button>
          )}
          {configured && (
            <button
              type="button"
              aria-label={`Clear DeepSeek API key for ${hostname}`}
              title={connected ? undefined : NOT_CONNECTED}
              disabled={!connected}
              onClick={() => requestConfirm({
                title: `Clear the DeepSeek API key on ${hostname}?`,
                body: <p>DeepSeek tabs on {hostname} need a key to start. You can set a new one at any time.</p>,
                confirmLabel: "Clear key",
                tone: "danger",
                onConfirm: () => {
                  // The row says "API key not configured" once it is sent.
                  if (!setMachineDeepseekConfig(machine.machineId, undefined, true)) setResult({ ok: false, text: NOT_SENT });
                },
              })}
              className={overviewQuietDangerButton}
            >
              Clear key
            </button>
          )}
        </div>
      </div>
      {open && (
        <form
          className="mt-2 flex flex-col gap-2 sm:flex-row sm:items-center"
          onSubmit={(event) => {
            event.preventDefault();
            if (!trimmed || !connected) return;
            const sent = setMachineDeepseekConfig(machine.machineId, trimmed, false);
            if (!sent) {
              // Kept open with the draft, so a retry is one click once the
              // page is connected again.
              setResult({ ok: false, text: NOT_SENT });
              return;
            }
            close();
            setResult({ ok: true, text: `Key sent to ${hostname}.` });
          }}
        >
          <input
            ref={field}
            type="password"
            autoComplete="new-password"
            spellCheck={false}
            aria-label={`DeepSeek API key for ${hostname}`}
            placeholder="Paste a new key"
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "Escape") return;
              event.preventDefault();
              close();
            }}
            className="h-8 min-w-0 flex-1 rounded border border-gray-300 bg-white px-2 text-base pointer-coarse:h-10 sm:text-sm dark:border-gray-600 dark:bg-gray-800"
          />
          <div className="flex justify-end gap-2">
            <button type="button" onClick={close} className={overviewButton}>Cancel</button>
            <button
              type="submit"
              aria-label={`Save DeepSeek API key for ${hostname}`}
              disabled={!trimmed || !connected}
              className={overviewPrimaryButton}
            >
              Save
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
