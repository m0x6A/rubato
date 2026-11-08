/** Ajv wiring for the schemas `rubato-protocol` exports (JSON Schema 2020-12). */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Ajv2020, type ValidateFunction } from "ajv/dist/2020.js";

const cache = new Map<string, ValidateFunction>();

const DATE_TIME = /^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function validator(schemaDir: string, file: string): ValidateFunction {
  const key = join(schemaDir, file);
  const hit = cache.get(key);
  if (hit) {
    return hit;
  }
  const ajv = new Ajv2020({ allErrors: true, strict: true });
  ajv.addFormat("date-time", DATE_TIME);
  ajv.addFormat("uuid", UUID);
  // schemars number formats carry no validation meaning for JSON; register them as accepted.
  for (const f of ["uint", "uint8", "uint16", "uint32", "uint64", "int8", "int16", "int32", "int64", "float", "double"]) {
    ajv.addFormat(f, true);
  }
  const schema = JSON.parse(readFileSync(key, "utf8")) as object;
  const fn = ajv.compile(schema);
  cache.set(key, fn);
  return fn;
}
