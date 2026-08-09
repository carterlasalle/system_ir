/** Worker: consumes transcript events and indexes them. */
import { redis } from "../shared/db";
import { prisma } from "../shared/db";

export async function indexTranscripts(): Promise<void> {
  const events = await redis.lrange("transcripts-ready", 0, -1);
  for (const ev of events) {
    const record = await prisma.transcript.findUnique({ where: { id: ev } });
    if (record) {
      await redis.set(`indexed:${ev}`, record.normalized_text);
    }
  }
}
