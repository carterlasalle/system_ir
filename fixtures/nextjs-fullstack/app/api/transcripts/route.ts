/** API route: serve transcript records. */
import { TranscriptStore } from "../../../lib/transcripts";

export async function GET(req: Request) {
  const url = new URL(req.url);
  const id = url.searchParams.get("id");
  const store = new TranscriptStore();
  const record = id ? await store.find(id) : await store.list();
  return Response.json({ transcript: record });
}

export async function POST(req: Request) {
  const body = await req.json();
  const store = new TranscriptStore();
  const record = await store.save(body.text);
  return Response.json({ transcript: record }, { status: 201 });
}
