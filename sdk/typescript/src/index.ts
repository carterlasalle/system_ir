/**
 * @scc/sdk — thin TypeScript SDK for the `scc` (System Context Compiler) CLI.
 *
 * Every method shells out to the `scc` binary with `--root <cwd>` and `--json`,
 * and parses the emitted context pack. The binary is resolved from the `bin`
 * option, then the `SCC_BIN` environment variable, then `scc` on PATH.
 */

import { spawn } from "node:child_process";

// trace:v1 id=impl.scc.sdk.typescript work=WORK-SCC-014 satisfies=REQ-SCC-IR

/** A compiled context pack emitted by the scc CLI. */
// trace:v1 id=impl.sdk-typescript-src-index.context-pack work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface ContextPack {
  kind: string;
  repository_revision: string;
  content: string;
  entity_ids: string[];
  evidence_summary: Record<string, number>;
  warnings: string[];
  tokens: number;
  budget: number;
  truncated: boolean;
}

// trace:v1 id=impl.sdk-typescript-src-index.scc-options work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface SCCOptions {
  /** Path to the scc binary (default: $SCC_BIN or `scc` on PATH). */
  bin?: string;
  /** Repository root passed as `--root` (default: process.cwd()). */
  cwd?: string;
}

// trace:v1 id=impl.sdk-typescript-src-index.task-context-options work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface TaskContextOptions {
  files?: string[];
  symbols?: string[];
  tokenBudget?: number;
}

// trace:v1 id=impl.sdk-typescript-src-index.task-context-artifact work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface TaskContextArtifact {
  /** The enriched task pack (`context task --json` → field `pack`). */
  // (fields documented inline below)
  pack: ContextPack;
  /** The task-personalized Surface delta (new relevant APIs vs the ledger). */
  delta: string;
  /** Entry ids the delta rendered (ledger recording). */
  delta_ids: string[];
  /** Actual token count of the complete rendered artifact (pack + delta). */
  token_count: number;
}

/** Result of `scc index`. */
// trace:v1 id=impl.sdk-typescript-src-index.index-result work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export interface IndexResult {
  ok: boolean;
}

/**
 * Client for the scc CLI. Each method runs the binary as a subprocess and
 * resolves with the parsed JSON result; a non-zero exit rejects with an
 * Error carrying the process's stderr.
 */
// trace:v1 id=impl.sdk-typescript-src-index-scc work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
export class SCC {
  // trace:v1 id=impl.sdk-typescript-src-index-scc.constructor work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  constructor(private opts: SCCOptions = {}) {}

  /** Resolve the scc binary: explicit option, then $SCC_BIN, then PATH. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.bin work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private get bin(): string {
    return this.opts.bin ?? process.env.SCC_BIN ?? "scc";
  }

  // trace:v1 id=impl.sdk-typescript-src-index-scc.cwd work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private get cwd(): string {
    return this.opts.cwd ?? process.cwd();
  }

  /**
   * Run `scc --root <cwd> <args>` and resolve with captured stdout/stderr.
   * Rejects on spawn failure or non-zero exit (message = trimmed stderr).
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.run work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private run(args: string[]): Promise<{ stdout: string; stderr: string }> {
    const { promise, resolve, reject } = Promise.withResolvers<{
      stdout: string;
      stderr: string;
    }>();
    const proc = spawn(this.bin, ["--root", this.cwd, ...args], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    proc.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString();
    });
    proc.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
    });
    proc.on("error", (err) => {
      reject(new Error(`failed to spawn ${this.bin}: ${err.message}`));
    });
    proc.on("close", (code) => {
      if (code === 0) {
        resolve({ stdout, stderr });
      } else {
        reject(new Error(stderr.trim() || `${this.bin} exited with code ${code}`));
      }
    });
    return promise;
  }

  /** Run a command that emits a JSON context pack on stdout. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.run-pack work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private async runPack(args: string[]): Promise<ContextPack> {
    const { stdout } = await this.run(args);
    return JSON.parse(stdout) as ContextPack;
  }

  /**
   * Run a command that emits arbitrary JSON on stdout and resolve it as `T`.
   * Used for commands whose JSON shape is not a flat ContextPack (e.g. the
   * task artifact `{pack, delta, delta_ids}`) — never casts the artifact to
   * a ContextPack.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.run-json work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  private async runJson<T>(args: string[]): Promise<T> {
    const { stdout } = await this.run(args);
    return JSON.parse(stdout) as T;
  }

  /** Compile the system overview capsule. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.system-overview work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async systemOverview(): Promise<ContextPack> {
    return this.runPack(["overview", "--json"]);
  }

  /**
   * Compile the complete task context artifact for a goal: the enriched
   * task pack plus the task-personalized Surface delta.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.task-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async taskContext(goal: string, opts?: TaskContextOptions): Promise<TaskContextArtifact> {
    const args = ["context", "task", goal];
    if (opts?.files && opts.files.length > 0) {
      args.push("--files", opts.files.join(" "));
    }
    if (opts?.symbols && opts.symbols.length > 0) {
      args.push("--symbols", opts.symbols.join(" "));
    }
    if (opts?.tokenBudget !== undefined) {
      args.push("--budget", String(opts.tokenBudget));
    }
    args.push("--json");
    return this.runJson<TaskContextArtifact>(args);
  }

  /** Compile the context pack for one component (by id or name). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.component-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async componentContext(id: string): Promise<ContextPack> {
    return this.runPack(["context", "component", id, "--json"]);
  }

  /** Compile the context pack for one flow (by id or name). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.flow-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async flowContext(id: string): Promise<ContextPack> {
    return this.runPack(["context", "flow", id, "--json"]);
  }

  /** Compile an impact analysis pack for a set of files/symbols. */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.impact-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async impactContext(files?: string[], symbols?: string[]): Promise<ContextPack> {
    const args: string[] = ["impact"];
    if (files && files.length > 0) {
      args.push(...files);
    }
    if (symbols && symbols.length > 0) {
      args.push("--symbols", symbols.join(" "));
    }
    args.push("--json");
    return this.runPack(args);
  }

  /**
   * Run the freshness/evidence verification. `scc verify` has no JSON mode,
   * so the pack is synthesized from its markdown output.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.verify-context work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async verifyContext(): Promise<ContextPack> {
    const { stdout } = await this.run(["verify"]);
    const revision = stdout.match(/^Revision:\s*(.+)$/m)?.[1]?.trim() ?? "";
    return {
      kind: "verify",
      repository_revision: revision,
      content: stdout,
      entity_ids: [],
      evidence_summary: {},
      warnings: [],
      tokens: 0,
      budget: 0,
      truncated: false,
    };
  }

  /**
   * Compile the fused session-startup artifact (Atlas + Surface + coverage +
   * omissions). `scc context startup` has no JSON mode, so the pack is
   * synthesized from its markdown output.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.context-startup work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async contextStartup(budget?: number): Promise<ContextPack> {
    const args = ["context", "startup"];
    if (budget !== undefined) {
      args.push("--budget", String(budget));
    }
    const { stdout } = await this.run(args);
    return {
      kind: "startup",
      repository_revision: "",
      content: stdout,
      entity_ids: [],
      evidence_summary: {},
      warnings: [],
      tokens: 0,
      budget: budget ?? 0,
      truncated: false,
    };
  }

  /**
   * Compile the System Surface Map, global or task-personalized. `scc
   * surface` has no JSON mode, so the pack is synthesized from its markdown
   * output.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.surface-map work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async surfaceMap(goal?: string, budget?: number): Promise<ContextPack> {
    const args: string[] = ["surface"];
    if (goal) {
      args.push("--task", goal);
    }
    if (budget !== undefined) {
      args.push("--budget", String(budget));
    }
    const { stdout } = await this.run(args);
    return {
      kind: "surface",
      repository_revision: "",
      content: stdout,
      entity_ids: [],
      evidence_summary: {},
      warnings: [],
      tokens: 0,
      budget: budget ?? 0,
      truncated: false,
    };
  }

  /**
   * Compile the Structural Source representation of files: pass `files`
   * explicitly, or a `goal` to select the task-matched files via the
   * PPR->Surface pipeline.
   * `scc context structural` has no JSON mode, so the pack is synthesized
   * from its markdown output.
   */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.structural-source work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async structuralSource(files?: string[], goal?: string, budget?: number): Promise<ContextPack> {
    const args: string[] = ["context", "structural"];
    if (files && files.length > 0) {
      args.push("--files", files.join(" "));
    }
    if (goal) {
      args.push("--task", goal);
    }
    if (budget !== undefined) {
      args.push("--budget", String(budget));
    }
    const { stdout } = await this.run(args);
    return {
      kind: "structural",
      repository_revision: "",
      content: stdout,
      entity_ids: [],
      evidence_summary: {},
      warnings: [],
      tokens: 0,
      budget: budget ?? 0,
      truncated: false,
    };
  }

  /** Index the repository (idempotent; incremental after the first run). */
  // trace:v1 id=impl.sdk-typescript-src-index-scc.index work=WORK-task-context-transport-parity satisfies=REQ-SCC-IR
  async index(): Promise<IndexResult> {
    await this.run(["index"]);
    return { ok: true };
  }
}
