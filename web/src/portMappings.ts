/** API contract for workspace port mappings.
 *
 * The concrete ApiClient methods are intentionally kept in api.ts by the
 * integration owner. This small interface lets the UI be developed and
 * tested without coupling it to the generated client implementation.
 */
export interface PortMapping {
  id: string;
  internal_port: number;
  display_name: string | null;
  status: "provisioning" | "ready" | "failed" | "deleting" | string;
  https_url: string | null;
  /** Managed mappings are created by a workspace capability and cannot be removed manually. */
  managed: boolean;
  purpose: "user" | "desktop";
  created_at?: number;
}

export interface CreatePortMappingInput {
  internal_port: number;
  display_name?: string;
}

export interface UpdatePortMappingInput {
  internal_port: number;
  /** `null` clears the name; the server trims and normalizes blank names. */
  display_name: string | null;
}

export interface PortMappingsApi {
  portMappings(workspaceId: string): Promise<PortMapping[]>;
  createPortMapping(workspaceId: string, input: CreatePortMappingInput): Promise<PortMapping>;
  updatePortMapping(workspaceId: string, mappingId: string, input: UpdatePortMappingInput): Promise<PortMapping>;
  deletePortMapping(workspaceId: string, mappingId: string): Promise<void>;
  /** Returns a short-lived URL suitable for opening in a new browser tab. */
  bootstrapPortMapping(workspaceId: string, mappingId: string): Promise<{ bootstrap_url: string }>;
}

export const PORT_MIN = 1;
export const PORT_MAX = 65_535;

/** Matches the backend Unicode scalar (code point) limit, not UTF-16 units. */
export const DISPLAY_NAME_MAX_CODE_POINTS = 80;

/**
 * Count code points so a name of 80 astral characters (for example emoji) is
 * accepted while 81 is rejected — the same rule the Rust backend applies.
 * A user-perceived emoji may be several code points; the contract is explicit.
 */
export function codePointLength(value: string): number {
  return Array.from(value).length;
}

export interface DisplayNameValidation {
  /** Trimmed value sent to the API; empty means no name. */
  value: string;
  length: number;
  tooLong: boolean;
}

/** Trim first, then measure; whitespace-only input is treated as cleared. */
export function validateDisplayName(raw: string): DisplayNameValidation {
  const value = raw.trim();
  const length = codePointLength(value);
  return { value, length, tooLong: length > DISPLAY_NAME_MAX_CODE_POINTS };
}

export function parseInternalPort(value: string): number | null {
  if (!/^\d+$/.test(value)) return null;
  const port = Number(value);
  const applicationPort = port === 80 || port === 443 || (port >= 1024 && port <= PORT_MAX);
  const reserved = [2222, 3389, 7681, 8080, 8081, 8443].includes(port);
  return Number.isSafeInteger(port) && applicationPort && !reserved ? port : null;
}

export function mappingUrl(mapping: PortMapping): string | null {
  return mapping.https_url ?? null;
}

/** Reject unsafe or credentialed destinations supplied by a bootstrap response. */
export function safeBootstrapUrl(value: string): string | null {
  try {
    const url = new URL(value);
    return url.protocol === "https:" && !url.username && !url.password ? url.href : null;
  } catch {
    return null;
  }
}
