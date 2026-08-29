import type { RJSFSchema } from "@rjsf/utils";

const FORBIDDEN_FIELD = /(?:^|[_\s-])(password|passwd|secret|token|credential|api[_\s-]?key|private[_\s-]?key)(?:$|[_\s-])|密码|口令|密钥|令牌|凭据|секрет|токен|пароль/iu;
const ALLOWED_KEYS = new Set([
  "$defs", "$ref", "additionalProperties", "allOf", "anyOf", "const", "default",
  "description", "else", "enum", "exclusiveMaximum", "exclusiveMinimum", "format", "if",
  "items", "maxItems", "maxLength", "maxProperties", "maximum", "minItems", "minLength",
  "minProperties", "minimum", "multipleOf", "not", "oneOf", "pattern", "properties",
  "propertyNames", "required", "then", "title", "type", "uniqueItems",
]);
const MAX_SCHEMA_BYTES = 64 * 1024;
const MAX_DEPTH = 32;
const MAX_PROPERTIES = 256;

export type SchemaRejectReason = "invalid" | "too_large" | "too_deep" | "too_many_fields" | "unsupported" | "sensitive";

export type SchemaCheck =
  | { ok: true; schema: RJSFSchema }
  | { ok: false; reason: SchemaRejectReason };

export function checkPluginSchema(value: unknown): SchemaCheck {
  if (!value || typeof value !== "object" || Array.isArray(value)) return { ok: false, reason: "invalid" };
  if (JSON.stringify(value).length > MAX_SCHEMA_BYTES) return { ok: false, reason: "too_large" };
  let properties = 0;

  function visit(node: unknown, depth: number, path: string[]): SchemaRejectReason | null {
    if (depth > MAX_DEPTH) return "too_deep";
    if (Array.isArray(node)) {
      for (const item of node) {
        const error = visit(item, depth + 1, path);
        if (error) return error;
      }
      return null;
    }
    if (!node || typeof node !== "object") return null;
    const object = node as Record<string, unknown>;
    for (const key of Object.keys(object)) {
      if (!ALLOWED_KEYS.has(key)) return "unsupported";
    }
    for (const key of ["title", "description"] as const) {
      const text = object[key];
      if (text !== undefined && (typeof text !== "string" || text.length > 2048 || /[<>\u0000-\u001f]/u.test(text))) return "unsupported";
    }
    if (typeof object.$ref === "string" && !object.$ref.startsWith("#/$defs/")) return "unsupported";
    if (typeof object.pattern === "string" && !safePattern(object.pattern)) return "unsupported";
    if (object.properties && (typeof object.properties !== "object" || Array.isArray(object.properties))) return "invalid";
    if (object.properties) {
      for (const [key, child] of Object.entries(object.properties as Record<string, unknown>)) {
        properties += 1;
        if (properties > MAX_PROPERTIES) return "too_many_fields";
        if (FORBIDDEN_FIELD.test([...path, key, metadataOf(child)].filter(Boolean).join(" "))) return "sensitive";
        const error = visit(child, depth + 1, [...path, key]);
        if (error) return error;
      }
    }
    if (object.$defs && (typeof object.$defs !== "object" || Array.isArray(object.$defs))) return "invalid";
    if (object.$defs) for (const child of Object.values(object.$defs as Record<string, unknown>)) {
      const error = visit(child, depth + 1, path);
      if (error) return error;
    }
    for (const [key, child] of Object.entries(object)) {
      if (key === "properties" || key === "$defs" || key === "enum" || key === "const" || key === "default") continue;
      const error = visit(child, depth + 1, path);
      if (error) return error;
    }
    return null;
  }

  const error = visit(value, 0, []);
  return error ? { ok: false, reason: error } : { ok: true, schema: value as RJSFSchema };
}

function metadataOf(value: unknown): string {
  if (!value || typeof value !== "object" || Array.isArray(value)) return "";
  const object = value as Record<string, unknown>;
  return [object.title, object.description].filter((text): text is string => typeof text === "string").join(" ");
}

function safePattern(pattern: string): boolean {
  return pattern.length <= 256
    && !/\\[1-9]|\(\?<?[=!]/u.test(pattern)
    && !/\([^)]*(?:[+*]|\{\d)[^)]*\)(?:[+*]|\{\d)/u.test(pattern)
    && validRegex(pattern);
}

function validRegex(pattern: string): boolean {
  try {
    new RegExp(pattern, "u");
    return true;
  } catch {
    return false;
  }
}

export function configurationKey(pluginId: string, scope: "installation" | "organization"): string {
  return `${pluginId}:${scope}`;
}
