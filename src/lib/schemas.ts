// Runtime schemas for everything the Rust layer returns. Data that fails
// validation is never rendered; the UI shows the validation error instead.
import { z } from "zod";

const bytes = z.number().int().nonnegative();
const finite = z.number().finite();
// Float rounding can put a fully busy core a hair over 100.
const percent = finite.min(0).max(100.5);
const rate = finite.nonnegative();

export const HostInfoSchema = z.object({
  hostname: z.string().nullable(),
  osVersion: z.string().nullable(),
  kernelVersion: z.string().nullable(),
  cpuBrand: z.string().nullable(),
  physicalCores: z.number().int().positive().nullable(),
  logicalCores: z.number().int().positive(),
});

export const ProcessSchema = z.object({
  pid: z.number().int().nonnegative(),
  name: z.string(),
  cpuPercent: finite.nonnegative().nullable(),
  memoryBytes: bytes,
});

export const SystemSnapshotSchema = z.object({
  timestampMs: z.number().int().nonnegative(),
  uptimeSecs: z.number().int().nonnegative(),
  host: HostInfoSchema,
  cpu: z.object({
    usagePercent: percent.nullable(),
    perCorePercent: z.array(percent),
    loadAverage: z.tuple([rate, rate, rate]),
  }),
  memory: z
    .object({
      totalBytes: bytes.positive(),
      usedBytes: bytes,
      freeBytes: bytes,
      swapTotalBytes: bytes,
      swapUsedBytes: bytes,
    })
    .refine((m) => m.usedBytes <= m.totalBytes, "used memory exceeds total")
    .refine((m) => m.swapUsedBytes <= m.swapTotalBytes, "used swap exceeds total"),
  disks: z.array(
    z
      .object({
        name: z.string(),
        mountPoint: z.string(),
        fileSystem: z.string(),
        totalBytes: bytes,
        availableBytes: bytes,
        removable: z.boolean(),
      })
      .refine((d) => d.availableBytes <= d.totalBytes, "disk free space exceeds capacity"),
  ),
  network: z.array(
    z.object({
      interface: z.string(),
      rxBytesPerSec: rate.nullable(),
      txBytesPerSec: rate.nullable(),
      totalRxBytes: bytes,
      totalTxBytes: bytes,
    }),
  ),
  processes: z
    .object({
      total: z.number().int().nonnegative(),
      unreadable: z.number().int().nonnegative(),
      topByCpu: z.array(ProcessSchema),
      topByMemory: z.array(ProcessSchema),
    })
    .refine((p) => p.unreadable <= p.total, "more unreadable processes than processes"),
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
    powerSource: z.string().nullable(),
    percent: z.number().int().min(0).max(100).nullable(),
    state: ChargeStateSchema,
    externalPower: z.boolean().nullable(),
    timeRemainingMin: z.number().int().nonnegative().nullable(),
    timeRemainingKind: z.enum(["untilEmpty", "untilFull"]).nullable(),
    cycleCount: z.number().int().nonnegative().nullable(),
    designCapacityMah: z.number().int().positive().nullable(),
    fullChargeCapacityMah: z.number().int().positive().nullable(),
    currentChargeMah: z.number().int().nonnegative().nullable(),
    rawCapacityPercent: finite.min(0).max(200).nullable(),
    appleMaxCapacityPercent: z.number().int().min(0).max(100).nullable(),
    condition: z.string().nullable(),
    voltageMv: z.number().int().positive().nullable(),
    amperageMa: z.number().int().nullable(),
    powerW: finite.nullable(),
    temperatureC: finite.min(-20).max(100).nullable(),
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
  chargeMah: z.number().int().nonnegative().nullable(),
  fullChargeMah: z.number().int().positive().nullable(),
  amperageMa: z.number().int().nullable(),
  voltageMv: z.number().int().positive().nullable(),
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
    discharge: DischargeEstimateSchema.nullable(),
    dischargeUnavailableReason: z.string().nullable(),
    sampleIntervalSecs: z.number().int().positive(),
    maxGapSecs: z.number().int().positive(),
    persistent: z.boolean(),
  })
  .refine(
    (h) => h.samples.every((s, i) => i === 0 || s.ts >= h.samples[i - 1].ts),
    "battery samples are out of order",
  );

export type SystemSnapshot = z.infer<typeof SystemSnapshotSchema>;
export type ProcessInfo = z.infer<typeof ProcessSchema>;
export type ChargeState = z.infer<typeof ChargeStateSchema>;
export type BatteryInfo = z.infer<typeof BatteryInfoSchema>;
export type BatterySample = z.infer<typeof BatterySampleSchema>;
export type DischargeEstimate = z.infer<typeof DischargeEstimateSchema>;
export type BatteryHistory = z.infer<typeof BatteryHistorySchema>;
