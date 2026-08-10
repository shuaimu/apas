import type { ErrorObject, ValidateFunction } from "ajv";
import Ajv2020 from "ajv/dist/2020.js";
import codeEventSchema from "../schema/code-event.schema.json" with { type: "json" };
import serverToWebSchema from "../schema/server-to-web.schema.json" with { type: "json" };
import webToServerSchema from "../schema/web-to-server.schema.json" with { type: "json" };

const ajv = new Ajv2020({ allErrors: true, strict: false });
ajv.addFormat("uuid", /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
ajv.addFormat("date-time", (value) => !Number.isNaN(Date.parse(value)));
ajv.addFormat("uint", { type: "number", validate: (value) => Number.isSafeInteger(value) && value >= 0 });
ajv.addFormat("uint16", { type: "number", validate: (value) => Number.isInteger(value) && value >= 0 && value <= 65_535 });
ajv.addFormat("uint32", { type: "number", validate: (value) => Number.isInteger(value) && value >= 0 && value <= 4_294_967_295 });
ajv.addFormat("uint64", { type: "number", validate: (value) => Number.isSafeInteger(value) && value >= 0 });
ajv.addFormat("int64", { type: "number", validate: Number.isSafeInteger });
ajv.addFormat("double", { type: "number", validate: Number.isFinite });
const validateWebToServer = ajv.compile(webToServerSchema) as ValidateFunction;
const validateServerToWeb = ajv.compile(serverToWebSchema) as ValidateFunction;
const validateCodeEventValue = ajv.compile(codeEventSchema) as ValidateFunction;

export interface ValidationResult {
  valid: boolean;
  errors: ErrorObject[];
}

function result(validator: ValidateFunction, value: unknown): ValidationResult {
  const valid = validator(value);
  return { valid, errors: valid ? [] : [...(validator.errors ?? [])] };
}

export function validateClientMessage(value: unknown): ValidationResult {
  return result(validateWebToServer, value);
}

export function validateServerMessage(value: unknown): ValidationResult {
  return result(validateServerToWeb, value);
}

export function validateCodeEvent(value: unknown): ValidationResult {
  return result(validateCodeEventValue, value);
}
