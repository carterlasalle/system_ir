/** Order repository: all order persistence. */
import { Client } from "pg";

export class OrderRepo {
  constructor(private db: Client) {}

  async list(): Promise<unknown[]> {
    const res = await this.db.query("SELECT * FROM orders ORDER BY id DESC");
    return res.rows;
  }

  async get(id: string): Promise<unknown> {
    const res = await this.db.query("SELECT * FROM orders WHERE id = $1", [id]);
    return res.rows[0];
  }

  async insert(order: unknown): Promise<unknown> {
    const res = await this.db.query("INSERT INTO orders (data) VALUES ($1) RETURNING id", [order]);
    return res.rows[0];
  }
}
