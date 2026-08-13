import express from "express";

export function health(): Record<string, boolean> {
  return { ok: true };
}

const app = express();
app.use(express.json());
app.get("/health", health);

const PORT = process.env.PORT;
export function start(): void {
  app.listen(PORT ?? "3000");
}

start();
