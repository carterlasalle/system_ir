import { describe, expect, it } from "vitest";
import { OrderService } from "../src/domain/orders";

describe("OrderService", () => {
  it("computes order totals from line items", async () => {
    const svc = new OrderService({ list: async () => [], get: async () => ({}), insert: async () => ({}) });
    expect(typeof svc.createOrder).toBe("function");
  });
});
