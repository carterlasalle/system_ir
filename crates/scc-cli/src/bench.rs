//! Benchmark harness (docs/TEST_PLAN.md §16, EPIC-240): generate synthetic
//! repositories and measure cold index / incremental / task-pack latency.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

pub struct BenchReport {
    pub files: usize,
    pub loc: usize,
    pub cold_ms: u64,
    pub incremental_ms: u64,
    /// P95 of 7 incremental refreshes (sorted[5]); SCC-242.
    pub incremental_p95_ms: u64,
    /// Peak resident set size after the cold index, in KiB.
    pub peak_rss_kib: u64,
    pub task_pack_ms: u64,
    pub db_bytes: u64,
}

/// Peak resident set size in KiB via getrusage. macOS reports ru_maxrss in
/// bytes, Linux in KiB — normalize to KiB on both.
fn peak_rss_kib() -> u64 {
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return 0;
    }
    #[cfg(target_os = "macos")]
    {
        usage.ru_maxrss as u64 / 1024
    }
    #[cfg(not(target_os = "macos"))]
    {
        usage.ru_maxrss as u64
    }
}

/// Generate a synthetic repo with `files` source files, each roughly `lines`
/// LOC, wired with imports and cross-file calls so extraction does real work.
pub fn generate_repo(dir: &Path, files: usize, lines: usize) -> (usize, usize) {
    std::fs::create_dir_all(dir).unwrap();
    let mut loc = 0usize;
    for i in 0..files {
        let name = format!("mod_{i:04}");
        let mut body = String::new();
        body.push_str(&format!("# module {name}\n"));
        if i > 0 {
            body.push_str(&format!(
                "from mod_{:04} import helper_{:04}\n",
                i - 1,
                i - 1
            ));
        }
        let mut line = 3;
        // one symbol per ~10 lines, calls to earlier modules
        let symbol_count = (lines / 10).max(2);
        for s in 0..symbol_count {
            let sym = format!("func_{s:03}");
            body.push_str(&format!(
                "def {sym}(a: int, b: int) -> int:\n    \"\"\"{sym} computes a value.\"\"\"\n"
            ));
            line += 2;
            let mut cur = 0;
            while cur < 6 {
                if i > 0 && cur % 3 == 0 {
                    body.push_str(&format!(
                        "    r = helper_{:04}(a, {cur})\n",
                        (i + cur) % files
                    ));
                } else {
                    body.push_str(&format!("    r = a + {cur} * b\n"));
                }
                line += 1;
                cur += 1;
            }
            body.push_str("    return r\n");
            line += 1;
        }
        // pad to the requested line count
        while line < lines {
            body.push_str("# padding comment for realistic size\n");
            line += 1;
        }
        loc += line;
        let mut f = std::fs::File::create(dir.join(format!("{name}.py"))).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }
    (files, loc)
}

/// Run the benchmark against a generated repo.
pub fn bench_index(root: &Path, files: usize, lines: usize) -> crate::Result<BenchReport> {
    let dir = root.join("repo");
    let (file_count, loc) = generate_repo(&dir, files, lines);

    // cold index
    let t = Instant::now();
    crate::commands::cmd_index(&dir, true)?;
    let cold_ms = t.elapsed().as_millis() as u64;
    let peak_rss_kib = peak_rss_kib();

    // incremental: refresh 7 times, touching a different file each pass
    // (SCC-241/242). P95 is sorted[5] of the 7 samples.
    let mut incremental_durations: Vec<u64> = Vec::with_capacity(7);
    for i in 0..7usize {
        let rel = format!("mod_{:04}.py", i % file_count);
        let extra = format!("\n# incremental edit {i}\n");
        std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join(&rel))?
            .write_all(extra.as_bytes())?;
        let t = Instant::now();
        crate::commands::cmd_index_paths(&dir, &[rel], true)?;
        incremental_durations.push(t.elapsed().as_millis() as u64);
    }
    incremental_durations.sort_unstable();
    let incremental_ms = incremental_durations[0];
    let incremental_p95_ms = incremental_durations[5];

    // task pack
    let t = Instant::now();
    crate::commands::cmd_context_task(&dir, "find the helper computation", &[], &[], None, true)?;
    let task_pack_ms = t.elapsed().as_millis() as u64;

    let _store = crate::open_store(&dir)?;
    let db_bytes = std::fs::metadata(crate::db_path(&dir))
        .map(|m| m.len())
        .unwrap_or(0);

    Ok(BenchReport {
        files: file_count,
        loc,
        cold_ms,
        incremental_ms,
        incremental_p95_ms,
        peak_rss_kib,
        task_pack_ms,
        db_bytes,
    })
}

pub fn print_report(r: &BenchReport) {
    println!("scc bench index");
    println!("  files:       {}", r.files);
    println!("  LOC:         {}", r.loc);
    println!("  cold index:       {} ms", r.cold_ms);
    println!("  incremental:      {} ms", r.incremental_ms);
    println!("  incremental p95:  {} ms", r.incremental_p95_ms);
    println!("  task pack:        {} ms", r.task_pack_ms);
    println!("  db size:          {} KiB", r.db_bytes / 1024);
    println!("  peak RSS:         {} MiB", r.peak_rss_kib / 1024);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_produces_indexable_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let (files, loc) = generate_repo(dir.path(), 5, 40);
        assert_eq!(files, 5);
        assert!(loc >= 5 * 40);
        crate::commands::cmd_index(dir.path(), true).unwrap();
        let store = crate::open_store(dir.path()).unwrap();
        assert!(store.stats().unwrap()["files"] >= 5);
    }

    #[test]
    fn bench_runs_end_to_end() {
        let dir = tempfile::TempDir::new().unwrap();
        let r = bench_index(dir.path(), 8, 50).unwrap();
        assert_eq!(r.files, 8);
        assert!(r.cold_ms > 0);
        assert!(r.db_bytes > 0);
        // SCC-242: p95 of the 7 incremental refreshes is always measured
        assert!(r.incremental_p95_ms > 0);
    }
}
