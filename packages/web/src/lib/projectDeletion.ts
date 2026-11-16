/**
 * Deleting a project on the APAS server: one request, then a wait until the
 * server reports the deletion done. Shared by the Delete project dialog and
 * the project-access dialog so both send and wait the same way.
 *
 * Nothing here retries a request. A caller that gets an error shows it and
 * sends again only when the user asks again.
 */

/** The HTTP API's base, as every REST call in the web resolves it. */
export const PROJECT_API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";

async function serverError(response: Response, fallback: string): Promise<Error> {
  const body = await response.json().catch(() => null) as { error?: string; message?: string } | null;
  return new Error(body?.error || body?.message || fallback);
}

/**
 * Ask the server to delete a project: one `POST /projects/:id/delete`.
 * Resolves once the server accepted it (`202`), which starts the deletion;
 * it is complete only when `waitForProjectDeletion` resolves. Rejects with
 * the server's own message when it refused, and with the fetch error when
 * the request did not get through.
 *
 * `confirmation` is the value the server checks, the project's id.
 */
export async function requestProjectDeletion(
  projectId: string,
  token: string | null,
  confirmation: string = projectId,
): Promise<void> {
  const response = await fetch(`${PROJECT_API_URL}/projects/${projectId}/delete`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify({ confirmation }),
  });
  if (response.status !== 202) throw await serverError(response, "Failed to delete project");
}

/** What `waitForProjectDeletion` rejects with when it stops asking. */
export const DELETION_STILL_IN_PROGRESS = "Deletion is still in progress. You can safely close this dialog.";

export interface DeletionWaitOptions {
  /** How many times to ask; 120 by default, a minute at the default pace. */
  attempts?: number;
  /** Pause before each ask, in milliseconds; 500 by default. */
  intervalMs?: number;
}

/**
 * Wait until the server reports a deletion done: `GET /projects/:id/deletion`
 * answers `404` once the project is gone, and `200` or `409` while the
 * cleanup runs. Rejects with the server's message on any other answer, and
 * with `DELETION_STILL_IN_PROGRESS` when the project is still there after
 * the last ask. The deletion itself goes on either way.
 */
export async function waitForProjectDeletion(
  projectId: string,
  token: string | null,
  options: DeletionWaitOptions = {},
): Promise<void> {
  const attempts = options.attempts ?? 120;
  const intervalMs = options.intervalMs ?? 500;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
    const response = await fetch(`${PROJECT_API_URL}/projects/${projectId}/deletion`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    if (response.status === 404) return;
    if (!response.ok && response.status !== 409) {
      throw await serverError(response, "Failed to check deletion status");
    }
  }
  throw new Error(DELETION_STILL_IN_PROGRESS);
}
