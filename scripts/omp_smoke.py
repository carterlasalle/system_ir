#!/usr/bin/env python3
"""CI smoke: `scc setup omp` discovery + generated-extension contracts.

The generated TypeScript is the product: clap rejects `--path`, compaction
must use `session.compacting`, and failed index must not be treated as
success. Optionally typechecks against `@oh-my-pi/pi-coding-agent` when npm
can install it.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


# trace:exempt reason=internal-helper
def scc_bin() -> str:
    env = os.environ.get("SCC_BIN")
    if env:
        return env
    debug = ROOT / "target" / "debug" / "scc"
    if debug.exists():
        return str(debug)
    found = shutil.which("scc")
    if found:
        return found
    sys.stderr.write("scc binary not found (build first or set SCC_BIN)\n")
        sys.exit(1)


# trace:exempt reason=internal-helper
def try_typecheck(ext_dir: Path) -> None:
    npm = shutil.which("npm")
    npx = shutil.which("npx")
    if not npm or not npx:
        print("omp smoke: npm not available; skipping OMP typings typecheck")
        return
    tsconfig = {
        "compilerOptions": {
            "target": "ES2022",
            "module": "ESNext",
            "moduleResolution": "bundler",
            "strict": True,
            "skipLibCheck": True,
            "noEmit": True,
            "types": [],
        },
        "include": ["index.ts"],
    }
    (ext_dir / "tsconfig.smoke.json").write_text(json.dumps(tsconfig))
    install = subprocess.run(
        [
            npm,
            "install",
            "--no-audit",
            "--no-fund",
            "--prefix",
            str(ext_dir),
            "--ignore-scripts",
            "typescript",
            "@oh-my-pi/pi-coding-agent",
        ],
        capture_output=True,
        text=True,
        timeout=180,
    )
    if install.returncode != 0:
        print(
            "omp smoke: @oh-my-pi/pi-coding-agent not installable; skipping typecheck "
            f"({(install.stderr or install.stdout)[:300]})"
        )
        return
    tsc = subprocess.run(
        [npx, "--prefix", str(ext_dir), "tsc", "-p", "tsconfig.smoke.json"],
        capture_output=True,
        text=True,
        timeout=120,
        cwd=str(ext_dir),
    )
    if tsc.returncode != 0:
        sys.stderr.write(
            "omp smoke: generated extension failed to typecheck against "
            f"@oh-my-pi/pi-coding-agent:\n{tsc.stdout}\n{tsc.stderr}\n"
        )
        raise SystemExit(1)
    print("omp smoke: typecheck vs @oh-my-pi/pi-coding-agent OK")


# trace:v1 id=ops.scc.omp-smoke work=WORK-p0-omp-integration-correctness-and-writable-benchmark-scientific-validit satisfies=REQ-implement-p0-omp-integration-correctness-and-writable-benchmark-scient
def main() -> int:
    bin_path = scc_bin()
    help_out = subprocess.run(
        [bin_path, "setup", "--help"], capture_output=True, text=True, timeout=30
    )
    if help_out.returncode != 0:
        sys.stderr.write(help_out.stderr)
        return 1
    if "omp" not in help_out.stdout.lower() and "Oh My Pi" not in help_out.stdout:
        sys.stderr.write(f"scc setup does not advertise omp:\n{help_out.stdout}\n")
        return 1

    with tempfile.TemporaryDirectory(prefix="scc-omp-smoke-") as tmp:
        repo = Path(tmp) / "repo"
        repo.mkdir()
        (repo / "hello.py").write_text("def hi():\n    return 1\n")
        proc = subprocess.run(
            [bin_path, "--root", str(repo), "setup", "omp"],
            capture_output=True,
            text=True,
            timeout=60,
        )
        if proc.returncode != 0:
            sys.stderr.write(proc.stderr or proc.stdout)
            return 1
        combined = proc.stdout + proc.stderr
        if "Restart OMP, then run `/extensions` to verify" not in combined:
            sys.stderr.write(f"setup must say restart then /extensions to verify:\n{combined}\n")
            return 1
        if "or run `/extensions` to reload" in combined:
            sys.stderr.write("/extensions must not be advertised as a reload\n")
            return 1

        ts = (repo / ".omp/extensions/scc/index.ts").read_text()
        checks = {
            "--paths": '"--paths"' in ts or '["index", "--paths"' in ts,
            "no --path": '"--path"' not in ts,
            "session.compacting": "session.compacting" in ts,
            "checkpoint --inject": "checkpoint" in ts and "--inject" in ts,
            "SCC_BIN": "SCC_BIN" in ts,
            "index failure loud": "index --paths failed" in ts or "did not succeed" in ts,
        }
        failed = [k for k, ok in checks.items() if not ok]
        if failed:
            sys.stderr.write(f"generated extension missing contracts {failed}\n")
            return 1

        skill = (repo / ".omp/skills/scc-system-context/SKILL.md").read_text()
        for name in ("system_context", "surface_map", "structural_source"):
            if f"`{name}`" not in skill:
                sys.stderr.write(f"skill does not teach {name}\n")
                return 1

        if os.environ.get("SCC_OMP_TYPECHECK", "1") != "0":
            try_typecheck(repo / ".omp/extensions/scc")

    print("omp smoke: setup discovery + generated extension contracts OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
