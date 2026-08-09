/**
 * Ingest pipeline: consume events, transcribe, normalize, store.
 */
import { Redis } from "redis-client";
import { transcribe } from "./asr/client";
import { normalize } from "../normalize/text";
import { redis } from "../cache";

export async function consume(message: { value: string }): Promise<void> {
  const raw = await transcribe(message.value);
  const normalized = normalize(raw);
  await redis.set(`transcript:${message.topic ?? "unknown"}`, normalized);
  await redis.publish("transcripts-ready", normalized);
}

export function subscribe(): void {
  redis.subscribe("radio-audio");
}
