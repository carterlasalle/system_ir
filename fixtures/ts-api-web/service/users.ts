/** User service: reads and writes user records through the db client. */
import { db } from "../db";

export async function listUsers(): Promise<Array<{ id: string; name: string }>> {
  const rows = await db.users.findMany({ select: { id: true, name: true } });
  return rows;
}

export async function createUser(data: { name: string }): Promise<{ id: string; name: string }> {
  const user = await db.users.create({ data });
  return user;
}
