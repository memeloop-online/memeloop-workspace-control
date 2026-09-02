/** Return the cursor that starts the requested page from a keyset history. */
export function previousWorkspaceCursor(history: readonly (string | null)[], pageNumber: number): string | null {
  if (pageNumber <= 1) return null;
  return history[pageNumber - 2] ?? null;
}

