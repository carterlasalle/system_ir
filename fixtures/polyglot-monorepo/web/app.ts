/** Web frontend: calls the payment service. */
import { api } from "./client";

export async function renderPayments(): Promise<string> {
  const res = await api.get("/payments");
  return res.data.map((p: { amount: number }) => `$${p.amount}`).join(", ");
}
