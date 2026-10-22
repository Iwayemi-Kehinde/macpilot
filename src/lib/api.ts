import { invoke } from "@tauri-apps/api/core";
import type { z } from "zod";
import {
  BatteryHistorySchema,
  BatteryInfoSchema,
  SystemSnapshotSchema,
  type BatteryHistory,
  type BatteryInfo,
  type SystemSnapshot,
} from "./schemas";

export class ValidationError extends Error {
  constructor(command: string, issues: z.ZodError["issues"]) {
    const detail = issues
      .slice(0, 3)
      .map((i) => `${i.path.join(".") || "(root)"}: ${i.message}`)
      .join("; ");
    super(`${command} returned data that failed validation (${detail})`);
    this.name = "ValidationError";
  }
}

export function validate<T>(command: string, schema: z.ZodType<T>, raw: unknown): T {
  const parsed = schema.safeParse(raw);
  if (!parsed.success) throw new ValidationError(command, parsed.error.issues);
  return parsed.data;
}

async function call<T>(command: string, schema: z.ZodType<T>, args?: Record<string, unknown>): Promise<T> {
  return validate(command, schema, await invoke(command, args));
}

export const getSystemSnapshot = (): Promise<SystemSnapshot> =>
  call("get_system_snapshot", SystemSnapshotSchema);

export const getBattery = (): Promise<BatteryInfo> => call("get_battery", BatteryInfoSchema);

export const getBatteryHistory = (hours: number): Promise<BatteryHistory> =>
  call("get_battery_history", BatteryHistorySchema, { hours });
