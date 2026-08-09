/** Order API handlers: delegates to the domain service. */
import { OrderService } from "../domain/orders";
import { createClient } from "../infra/db";

export async function listOrders(): Promise<unknown[]> {
  const svc = new OrderService(createClient());
  return svc.findAll();
}

export async function getOrder(id: string): Promise<unknown> {
  const svc = new OrderService(createClient());
  return svc.findById(id);
}
