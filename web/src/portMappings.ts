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
  created_at?: number;
}

export interface CreatePortMappingInput {
  internal_port: number;
  display_name?: string;
}

export interface PortMappingsApi {
  portMappings(workspaceId: string): Promise<PortMapping[]>;
  createPortMapping(workspaceId: string, input: CreatePortMappingInput): Promise<PortMapping>;
  deletePortMapping(workspaceId: string, mappingId: string): Promise<void>;
  /** Returns a short-lived URL suitable for opening in a new browser tab. */
  bootstrapPortMapping(workspaceId: string, mappingId: string): Promise<{ bootstrap_url: string }>;
}

export const PORT_MIN = 1;
export const PORT_MAX = 65_535;

export function parseInternalPort(value: string): number | null {
  if (!/^\d+$/.test(value)) return null;
  const port = Number(value);
  const applicationPort = port === 80 || port === 443 || (port >= 1024 && port <= PORT_MAX);
  const reserved = [2222, 7681, 8080, 8081, 8443].includes(port);
  return Number.isSafeInteger(port) && applicationPort && !reserved ? port : null;
}

export function mappingUrl(mapping: PortMapping): string | null {
  return mapping.https_url ?? null;
}
