import {
  createErrorHandler,
  deepEquals,
  toErrorSchema,
  unwrapErrorHandler,
  validationDataMerge,
  type CustomValidator,
  type ErrorTransformer,
  type RJSFSchema,
  type RJSFValidationError,
  type UiSchema,
  type ValidationData,
  type ValidatorType,
} from "@rjsf/utils";

type Schema = RJSFSchema | boolean;

function addError(
  errors: RJSFValidationError[],
  name: string,
  message: string,
  path: Array<string | number>,
  schemaPath: string,
  params: Record<string, unknown> = {},
) {
  const property = path.length ? `.${path.join(".")}` : ".";
  errors.push({ name, message, property, schemaPath, params, stack: `${property} ${message}` });
}

function matchesType(type: string, value: unknown): boolean {
  if (type === "null") return value === null;
  if (type === "array") return Array.isArray(value);
  if (type === "object") return value !== null && typeof value === "object" && !Array.isArray(value);
  if (type === "integer") return typeof value === "number" && Number.isInteger(value);
  if (type === "number") return typeof value === "number" && Number.isFinite(value);
  return typeof value === type;
}

function resolve(root: Schema, reference: string): Schema | undefined {
  if (!reference.startsWith("#/$defs/")) return undefined;
  let value: unknown = root;
  for (const raw of reference.slice(2).split("/")) {
    if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
    const key = raw.replaceAll("~1", "/").replaceAll("~0", "~");
    value = (value as Record<string, unknown>)[key];
  }
  return typeof value === "boolean" || (value !== null && typeof value === "object")
    ? value as Schema
    : undefined;
}

function validate(
  schema: Schema,
  value: unknown,
  root: Schema,
  path: Array<string | number>,
  schemaPath: string,
  errors: RJSFValidationError[],
  depth = 0,
): boolean {
  const before = errors.length;
  if (schema === true) return true;
  if (schema === false) {
    addError(errors, "false schema", "is not allowed", path, schemaPath);
    return false;
  }
  if (depth > 32) {
    addError(errors, "$ref", "schema nesting is too deep", path, schemaPath);
    return false;
  }
  if (typeof schema.$ref === "string") {
    const resolved = resolve(root, schema.$ref);
    if (!resolved) addError(errors, "$ref", "unsupported schema reference", path, `${schemaPath}/$ref`);
    else validate(resolved, value, root, path, schema.$ref, errors, depth + 1);
  }
  if (value === undefined) return errors.length === before;

  const types = Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : [];
  if (types.length && !types.some((type) => matchesType(String(type), value))) {
    addError(errors, "type", `must be ${types.join(" or ")}`, path, `${schemaPath}/type`, { type: schema.type });
    return false;
  }
  if (schema.const !== undefined && !deepEquals(value, schema.const)) addError(errors, "const", "must equal the configured value", path, `${schemaPath}/const`);
  if (schema.enum && !schema.enum.some((item) => deepEquals(value, item))) addError(errors, "enum", "must be one of the allowed values", path, `${schemaPath}/enum`);

  schema.allOf?.forEach((item, index) => validate(item, value, root, path, `${schemaPath}/allOf/${index}`, errors, depth + 1));
  if (schema.anyOf && !schema.anyOf.some((item, index) => validate(item, value, root, path, `${schemaPath}/anyOf/${index}`, [], depth + 1))) addError(errors, "anyOf", "must match at least one allowed shape", path, `${schemaPath}/anyOf`);
  if (schema.oneOf && schema.oneOf.filter((item, index) => validate(item, value, root, path, `${schemaPath}/oneOf/${index}`, [], depth + 1)).length !== 1) addError(errors, "oneOf", "must match exactly one allowed shape", path, `${schemaPath}/oneOf`);
  if (schema.not && validate(schema.not, value, root, path, `${schemaPath}/not`, [], depth + 1)) addError(errors, "not", "matches a forbidden shape", path, `${schemaPath}/not`);
  if (schema.if) {
    const condition = validate(schema.if, value, root, path, `${schemaPath}/if`, [], depth + 1);
    const branch = condition ? schema.then : schema.else;
    if (branch) validate(branch, value, root, path, `${schemaPath}/${condition ? "then" : "else"}`, errors, depth + 1);
  }

  if (typeof value === "string") {
    const length = [...value].length;
    if (schema.minLength !== undefined && length < schema.minLength) addError(errors, "minLength", `must contain at least ${schema.minLength} characters`, path, `${schemaPath}/minLength`);
    if (schema.maxLength !== undefined && length > schema.maxLength) addError(errors, "maxLength", `must contain at most ${schema.maxLength} characters`, path, `${schemaPath}/maxLength`);
    if (schema.pattern && !safePattern(schema.pattern, value)) addError(errors, "pattern", "does not match the required format", path, `${schemaPath}/pattern`);
    if (schema.format && !validFormat(schema.format, value)) addError(errors, "format", `must be a valid ${schema.format}`, path, `${schemaPath}/format`);
  }
  if (typeof value === "number") {
    if (schema.minimum !== undefined && value < schema.minimum) addError(errors, "minimum", `must be at least ${schema.minimum}`, path, `${schemaPath}/minimum`);
    if (schema.maximum !== undefined && value > schema.maximum) addError(errors, "maximum", `must be at most ${schema.maximum}`, path, `${schemaPath}/maximum`);
    if (schema.exclusiveMinimum !== undefined && value <= Number(schema.exclusiveMinimum)) addError(errors, "exclusiveMinimum", `must be greater than ${schema.exclusiveMinimum}`, path, `${schemaPath}/exclusiveMinimum`);
    if (schema.exclusiveMaximum !== undefined && value >= Number(schema.exclusiveMaximum)) addError(errors, "exclusiveMaximum", `must be less than ${schema.exclusiveMaximum}`, path, `${schemaPath}/exclusiveMaximum`);
    if (schema.multipleOf !== undefined && Math.abs(value / schema.multipleOf - Math.round(value / schema.multipleOf)) > 1e-9) addError(errors, "multipleOf", `must be a multiple of ${schema.multipleOf}`, path, `${schemaPath}/multipleOf`);
  }
  if (Array.isArray(value)) {
    if (schema.minItems !== undefined && value.length < schema.minItems) addError(errors, "minItems", `must contain at least ${schema.minItems} items`, path, `${schemaPath}/minItems`);
    if (schema.maxItems !== undefined && value.length > schema.maxItems) addError(errors, "maxItems", `must contain at most ${schema.maxItems} items`, path, `${schemaPath}/maxItems`);
    if (schema.uniqueItems && new Set(value.map(canonicalKey)).size !== value.length) addError(errors, "uniqueItems", "must not contain duplicates", path, `${schemaPath}/uniqueItems`);
    if (schema.items && !Array.isArray(schema.items)) value.forEach((item, index) => validate(schema.items as Schema, item, root, [...path, index], `${schemaPath}/items`, errors, depth + 1));
  }
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    const object = value as Record<string, unknown>;
    const entries = Object.entries(object);
    if (schema.minProperties !== undefined && entries.length < schema.minProperties) addError(errors, "minProperties", `must contain at least ${schema.minProperties} properties`, path, `${schemaPath}/minProperties`);
    if (schema.maxProperties !== undefined && entries.length > schema.maxProperties) addError(errors, "maxProperties", `must contain at most ${schema.maxProperties} properties`, path, `${schemaPath}/maxProperties`);
    for (const required of schema.required ?? []) if (!(required in object)) addError(errors, "required", "is required", [...path, required], `${schemaPath}/required`);
    for (const [key, child] of Object.entries(schema.properties ?? {})) if (object[key] !== undefined) validate(child as Schema, object[key], root, [...path, key], `${schemaPath}/properties/${key}`, errors, depth + 1);
    if (schema.additionalProperties === false) for (const key of Object.keys(object)) if (!(key in (schema.properties ?? {}))) addError(errors, "additionalProperties", "is not an allowed field", [...path, key], `${schemaPath}/additionalProperties`);
    else if (schema.additionalProperties && typeof schema.additionalProperties === "object") for (const [key, item] of entries) if (!(key in (schema.properties ?? {}))) validate(schema.additionalProperties as Schema, item, root, [...path, key], `${schemaPath}/additionalProperties`, errors, depth + 1);
  }
  return errors.length === before;
}

function safePattern(pattern: string, value: string): boolean {
  if (pattern.length > 256 || value.length > 4096 || /\\[1-9]|\(\?<?[=!]/u.test(pattern)) return false;
  if (/\([^)]*(?:[+*]|\{\d)[^)]*\)(?:[+*]|\{\d)/u.test(pattern)) return false;
  try { return new RegExp(pattern, "u").test(value); } catch { return false; }
}

function validFormat(format: string, value: string): boolean {
  if (format === "uuid") return /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu.test(value);
  if (format === "email") return /^[^\s@]+@[^\s@]+\.[^\s@]+$/u.test(value);
  if (format === "date-time") return !Number.isNaN(Date.parse(value));
  if (format === "uri" || format === "uri-reference") {
    if (!/^[\x21-\x7e]*$/u.test(value) || /%(?![0-9a-f]{2})/iu.test(value)) return false;
    try { new URL(value, format === "uri" ? undefined : "https://schema.invalid/"); return true; } catch { return false; }
  }
  return true;
}

function canonicalKey(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalKey).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, item]) => `${JSON.stringify(key)}:${canonicalKey(item)}`).join(",")}}`;
  return JSON.stringify(value) ?? "undefined";
}

class SafeValidator implements ValidatorType {
  rawValidation<Result = RJSFValidationError>(schema: RJSFSchema, formData?: unknown) {
    const errors: RJSFValidationError[] = [];
    validate(schema, formData, schema, [], "#", errors);
    return { errors: errors as Result[] };
  }

  isValid(schema: RJSFSchema, formData: unknown, rootSchema: RJSFSchema) {
    return validate(schema, formData, rootSchema, [], "#", []);
  }

  validateFormData(formData: unknown, schema: RJSFSchema, customValidate?: CustomValidator, transformErrors?: ErrorTransformer, uiSchema?: UiSchema): ValidationData<unknown> {
    let errors = this.rawValidation<RJSFValidationError>(schema, formData).errors ?? [];
    if (transformErrors) errors = transformErrors(errors, uiSchema);
    const result = { errors, errorSchema: toErrorSchema(errors) };
    if (!customValidate) return result;
    const custom = customValidate(formData, createErrorHandler(formData), uiSchema, result.errorSchema);
    return validationDataMerge(result, unwrapErrorHandler(custom));
  }
}

export const safeValidator = new SafeValidator();
