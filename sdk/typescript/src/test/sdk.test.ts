/**
 * SDK integration tests against a real `scc` binary and a throwaway fixture
 * repository. Skipped when no `scc` binary is available (via $SCC_BIN or PATH).
 */

import { test, after } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { SCC } from "../index";

function resolveSccBin(): string | null {
  const fromEnv = process.env.SCC_BIN;
  if (fromEnv) return fromEnv;
  const probe = spawnSync("scc", ["--version"], { stdio: "ignore" });
  return probe.status === 0 ? "scc" : null;
}

const sccBin = resolveSccBin();
const skip = sccBin === null;
const skipReason = "scc binary not found (set SCC_BIN or add scc to PATH)";

const fixtureDir = skip ? null : mkdtempSync(join(tmpdir(), "scc-sdk-ts-"));

if (!skip) {
  const gitDir = join(fixtureDir!, ".git");
  mkdirSync(gitDir);
  writeFileSync(join(fixtureDir!, "a.py"), [
    "def add(a, b):",
    "    return a + b",
    "",
    "class Calculator:",
    "    def multiply(self, x, y):",
    "        return x * y",
    "",
  ].join("\n"));
  writeFileSync(join(fixtureDir!, "b.py"), [
    "from a import add, Calculator",
    "",
    "result = add(1, 2)",
    "calc = Calculator()",
    "prod = calc.multiply(3, 4)",
    "",
  ].join("\n"));
}

after(() => {
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

function scc(cwd?: string): SCC {
  return new SCC({ bin: sccBin ?? undefined, cwd: cwd ?? fixtureDir ?? undefined });
}

test("index() builds the index and reports ok", { skip: skip ? skipReason : false }, async () => {
  const result = await scc().index();
  assert.deepEqual(result, { ok: true });
});

test("systemOverview() content identifies the repository", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().systemOverview();
  assert.equal(pack.kind, "overview");
  assert.match(pack.content, /IDENTITY/);
  assert.ok(Array.isArray(pack.entity_ids));
});

test("taskContext() returns a pack with entity_ids array", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().taskContext("transcript");
  assert.equal(pack.kind, "task");
  assert.ok(Array.isArray(pack.entity_ids));
  assert.match(pack.content, /Goal: transcript/);
});

test("taskContext() honors files/symbols/tokenBudget options", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().taskContext("add numbers", {
    files: ["a.py", "b.py"],
    symbols: ["add"],
    tokenBudget: 500,
  });
  assert.match(pack.content, /Explicit files: a\.py, b\.py/);
  assert.match(pack.content, /Explicit symbols: add/);
});

test("componentContext() resolves a component", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().componentContext("root");
  assert.equal(pack.kind, "component");
  assert.ok(pack.entity_ids.length > 0);
  assert.match(pack.content, /RESPONSIBILITY/);
});

test("flowContext() resolves a flow", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().flowContext("architecture");
  assert.equal(pack.kind, "flow");
  assert.ok(pack.entity_ids.length > 0);
  assert.match(pack.content, /STEPS/);
});

test("impactContext() returns an impact pack", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().impactContext(["a.py"], ["add"]);
  assert.equal(pack.kind, "impact");
  assert.match(pack.content, /RISK/);
});

test("verifyContext() content reports freshness", { skip: skip ? skipReason : false }, async () => {
  const pack = await scc().verifyContext();
  assert.equal(pack.kind, "verify");
  assert.match(pack.content, /FRESHNESS/);
});

test("non-zero scc exit rejects with stderr", { skip: skip ? skipReason : false }, async () => {
  // A fake binary that fails with a distinctive stderr message.
  const fakeBin = join(fixtureDir!, "fake-scc");
  writeFileSync(fakeBin, "#!/bin/sh\necho 'boom: exploded' >&2\nexit 3\n");
  chmodSync(fakeBin, 0o755);
  const client = new SCC({ bin: fakeBin, cwd: fixtureDir! });
  await assert.rejects(() => client.systemOverview(), /boom: exploded/);
});
