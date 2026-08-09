/** Web view: renders the user list from the API response. */
import { api } from "./client";

export async function renderUsers(): Promise<string> {
  const res = await api.get("/api/users");
  return res.data.users.map((u: { name: string }) => u.name).join(", ");
}
