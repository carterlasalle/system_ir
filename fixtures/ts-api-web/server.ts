/**
 * Users API: serves user records. The response contract field is `name`.
 */
import express from "express";
import { listUsers, createUser } from "./service/users";
import { db } from "./db";

const app = express();

app.get("/api/users", async (req, res) => {
  const users = await listUsers();
  res.json({ users });
});

app.post("/api/users", async (req, res) => {
  const user = await createUser(req.body);
  res.status(201).json({ user });
});

app.get("/health", (req, res) => {
  res.json({ ok: true });
});

export default app;
