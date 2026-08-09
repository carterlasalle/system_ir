/**
 * API: exposes transcripts with a response contract field `transcript`.
 * Frontend and worker both consume this field.
 */
import { Router } from "express";
import { prisma } from "../shared/db";

const router = Router();

router.get("/api/transcripts/:id", async (req, res) => {
  const record = await prisma.transcript.findUnique({ where: { id: req.params.id } });
  res.json({ id: record.id, transcript: record.raw_text, normalizedTranscript: record.normalized_text });
});

router.post("/api/transcripts", async (req, res) => {
  const record = await prisma.transcript.create({ data: req.body });
  res.status(201).json({ id: record.id, transcript: record.raw_text });
});

export default router;
