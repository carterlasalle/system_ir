/**
 * HTTP server: routes to handlers.
 */
import express from "express";
import { listOrders, getOrder } from "./orders";
import { web } from "../web/render";

const app = express();

app.get("/api/orders", async (req, res) => {
  const orders = await listOrders();
  res.json({ orders });
});

app.get("/api/orders/:id", async (req, res) => {
  const order = await getOrder(req.params.id);
  res.json({ order });
});

app.get("/", async (req, res) => {
  res.send(await web.renderHome());
});

export default app;
