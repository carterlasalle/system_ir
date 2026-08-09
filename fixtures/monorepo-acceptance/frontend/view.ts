/** Frontend: renders the transcript from the API response. */
import { api } from "./client";

export async function renderTranscript(id: string): Promise<string> {
  const res = await api.get(`/api/transcripts/${id}`);
  return res.data.transcript; // response contract: `transcript` field
}
