/**
 * Queue worker entrypoint: consumes radio audio events from Kafka,
 * transcribes via an external ASR API, and persists normalized results.
 */
import { Kafka } from "kafka-client";
import { consume, subscribe } from "./ingest";
import { redis } from "./cache";

export function main(): void {
  const kafka = new Kafka({ brokers: ["localhost:9092"] });
  const consumer = kafka.consumer({ groupId: "transcripts" });
  consumer.subscribe({ topic: "radio-audio" });
  consumer.run({ eachMessage: consume });
}

if (require.main === module) {
  main();
}
