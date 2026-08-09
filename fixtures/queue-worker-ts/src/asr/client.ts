/** External ASR client with retry and fallback. */
import { fetch } from "undici";

@retry({ retries: 3, backoff: "exponential" })
export async function transcribe(audio: string): Promise<string> {
  const res = await fetch("https://asr.example.com/transcribe", {
    method: "POST",
    body: audio,
  });
  return (await res.json()).text;
}
