/**
 * Order domain: business rules live here.
 * Invariant: an order's total must equal the sum of its line totals.
 */
import { OrderRepo } from "../infra/repo";

export class OrderService {
  constructor(private repo: OrderRepo) {}

  async findAll(): Promise<unknown[]> {
    return this.repo.list();
  }

  async findById(id: string): Promise<unknown> {
    return this.repo.get(id);
  }

  async createOrder(lines: Array<{ price: number; qty: number }>): Promise<unknown> {
    const total = lines.reduce((acc, l) => acc + l.price * l.qty, 0);
    return this.repo.insert({ lines, total });
  }
}
