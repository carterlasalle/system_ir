/** Database client factory. */
import { Client } from "pg";

export function createClient(): Client {
  return new Client({ connectionString: process.env.DATABASE_URL });
}
