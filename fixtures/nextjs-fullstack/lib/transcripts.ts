/** Transcript store: owns all transcript persistence. */
import { sql } from "./db";

export class TranscriptStore {
  async find(id: string): Promise<unknown> {
    const rows = await sql`SELECT * FROM transcripts WHERE id = ${id}`;
    return rows[0];
  }

  async list(): Promise<unknown[]> {
    return sql`SELECT * FROM transcripts ORDER BY created_at DESC`;
  }

  async save(text: string): Promise<unknown> {
    const rows = await sql`INSERT INTO transcripts (text) VALUES (${text}) RETURNING *`;
    return rows[0];
  }
}
