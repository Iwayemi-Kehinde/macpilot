// Runtime schemas for everything the Rust layer returns. Data that fails
// validation is never rendered; the UI shows the validation error instead.
import { z } from "zod";

const bytes = z.number().int().nonnegative();
const finite = z.number().finite();
// Float rounding can put a fully busy core a hair over 100.
const percent = finite.min(0).max(100.5);
const rate = finite.nonnegative();
const watts = finite.min(-1000).max(1000);

export const AppUsageSchema = z.object({
  name: z.string(),
  processCount: z.number().int().positive(),
  /** Processes of this user: macOS reports memory and power only for these. */
  ownProcessCount: z.number().int().nonnegative(),
  /** Percent of one core, summed over the app's processes. */
  cpuPercent: rate.nullable(),
  powerW: rate.nullable(),
  memoryBytes: bytes.nullable(),
});

export const SystemSnapshotSchema = z.object({
  timestampMs: z.number().int().nonnegative(),
  uptimeSecs: z.number().int().nonnegative(),
  host: z.object({
    hostname: z.string().nullable(),
    osVersion: z.string().nullable(),
    cpuBrand: z.string().nullable(),
    logicalCores: z.number().int().positive(),
  }),
  cpu: z.object({
    usagePercent: percent.nullable(),
    thermal: z.enum(["nominal", "moderate", "heavy", "trapping", "sleeping"]).nullable(),
  }),
  memory: z
    .object({
      totalBytes: bytes.positive(),
      usedBytes: bytes,
      pressure: z.enum(["normal", "warning", "critical"]).nullable(),
      swapTotalBytes: bytes,
      swapUsedBytes: bytes,
    })
    .refine((m) => m.usedBytes <= m.totalBytes, "used memory exceeds total")
    .refine((m) => m.swapUsedBytes <= m.swapTotalBytes, "used swap exceeds total"),
  disk: z
    .object({ name: z.string(), totalBytes: bytes, availableBytes: bytes })
    .refine((d) => d.availableBytes <= d.totalBytes, "disk free space exceeds capacity")
    .nullable(),
  network: z.object({ rxBytesPerSec: rate.nullable(), txBytesPerSec: rate.nullable() }),
  apps: z
    .object({
      totalProcesses: z.number().int().nonnegative(),
      systemProcesses: z.number().int().nonnegative(),
      error: z.string().nullable(),
      apps: z.array(AppUsageSchema.refine((a) => a.ownProcessCount <= a.processCount, "more own processes than processes")),
    })
    .refine((a) => a.systemProcesses <= a.totalProcesses, "more system processes than processes"),
});

export const ChargeStateSchema = z.enum([
  "charging",
  "discharging",
  "charged",
  "acAttached",
  "finishingCharge",
  "unknown",
]);

export const BatteryInfoSchema = z
  .object({
    present: z.boolean(),
    percent: z.number().int().min(0).max(100).nullable(),
    state: ChargeStateSchema,
    externalPower: z.boolean().nullable(),
    timeRemainingMin: z.number().int().nonnegative().nullable(),
    timeRemainingKind: z.enum(["untilEmpty", "untilFull"]).nullable(),
    cycleCount: z.number().int().nonnegative().nullable(),
    appleMaxCapacityPercent: z.number().int().min(0).max(100).nullable(),
    condition: z.string().nullable(),
    temperatureC: finite.min(-20).max(100).nullable(),
    /** What the whole Mac draws, screen included. */
    systemPowerW: watts.min(0).nullable(),
    adapterPowerW: watts.min(0).nullable(),
    /** Into the battery while positive, out of it while negative. */
    batteryPowerW: watts.nullable(),
    adapterRatingW: z.number().int().positive().nullable(),
    /** On battery: the gauge's present current as points of the shown percentage per hour. */
    drainPercentPerHour: rate.nullable(),
    drainCalibrated: z.boolean(),
    notes: z.array(z.string()),
  })
  .refine(
    (b) => b.timeRemainingMin === null || b.timeRemainingKind !== null,
    "time remaining given without saying whether it is to empty or to full",
  );

export const BatterySampleSchema = z.object({
  ts: z.number().int().positive(),
  percent: z.number().int().min(0).max(100),
  charging: z.boolean(),
  externalPower: z.boolean(),
});

export const DischargeEstimateSchema = z.object({
  percentPerHour: finite,
  averageMa: finite.nullable(),
  windowMinutes: finite.nonnegative(),
  sampleCount: z.number().int().positive(),
  projectedMinutesToEmpty: finite.nonnegative().nullable(),
  confidence: z.enum(["low", "medium", "high"]),
  confidenceReason: z.string(),
});

export const BatteryHistorySchema = z
  .object({
    samples: z.array(BatterySampleSchema),
    /** The drain averaged over the current run on battery. */
    average: DischargeEstimateSchema.nullable(),
    averageUnavailableReason: z.string().nullable(),
    usual: z.object({ percentPerHour: rate, hours: rate }).nullable(),
    sampleIntervalSecs: z.number().int().positive(),
    maxGapSecs: z.number().int().positive(),
    persistent: z.boolean(),
  })
  .refine(
    (h) => h.samples.every((s, i) => i === 0 || s.ts >= h.samples[i - 1].ts),
    "battery samples are out of order",
  );

export const SleepBlockerSchema = z.object({
  pid: z.number().int().nonnegative(),
  process: z.string(),
  forPid: z.number().int().nonnegative().nullable(),
  app: z.string(),
  keeps: z.enum(["display", "system"]),
  reason: z.string(),
  heldSecs: z.number().int().nonnegative().nullable(),
  endsInSecs: z.number().int().nonnegative().nullable(),
});

export const ListenerSchema = z.object({
  port: z.number().int().min(0).max(65535),
  pid: z.number().int().nonnegative(),
  process: z.string(),
  app: z.string(),
  addresses: z.array(z.string()).min(1),
  /** Reachable from other devices on the network. */
  network: z.boolean(),
  folder: z.string().nullable(),
  path: z.string().nullable(),
});

export type SystemSnapshot = z.infer<typeof SystemSnapshotSchema>;
export type AppUsage = z.infer<typeof AppUsageSchema>;
export type ChargeState = z.infer<typeof ChargeStateSchema>;
export type BatteryInfo = z.infer<typeof BatteryInfoSchema>;
export type BatterySample = z.infer<typeof BatterySampleSchema>;
export type DischargeEstimate = z.infer<typeof DischargeEstimateSchema>;
export type BatteryHistory = z.infer<typeof BatteryHistorySchema>;
export type SleepBlocker = z.infer<typeof SleepBlockerSchema>;
export type Listener = z.infer<typeof ListenerSchema>;
