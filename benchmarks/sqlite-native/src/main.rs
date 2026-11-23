//! Native Rust SQLite comparison CLI. Benchmark-only, never a database
//! service or proof of speed superiority over other database systems.
use rystemdb_bench_sqlite::{load_fixture, run_fixture, validate_report};
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

struct TempRoot(PathBuf);
impl TempRoot {
    fn new() -> Result<Self> {
        let ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(
            format!("rystem-native-sqlite-{}-{ns}", std::process::id()),
        );
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

fn run() -> Result<()> {
    let mut fixture_path = None;
    let mut output_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--fixture" if fixture_path.is_none() => fixture_path = Some(PathBuf::from(value)),
            "--output" if output_path.is_none() => output_path = Some(PathBuf::from(value)),
            _ => return Err(format!("invalid or duplicate option {flag}").into()),
        }
    }
    let fixture_path = fixture_path.ok_or(
        "usage: rystemdb-bench-sqlite --fixture storage-fixture.ndjson --output sqlite-rust-benchmark.ndjson"
    )?;
    let output_path = output_path.ok_or("missing required --output")?;
    let fixture = load_fixture(&fixture_path)?;
    let root = TempRoot::new()?;
    let report = run_fixture(&fixture, &root.0.join("bench.sqlite3"))?;
    validate_report(&report, &fixture)?;
    let ndjson = report.iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?.join("\n") + "\n";
    print!("{ndjson}");
    fs::write(output_path, ndjson)?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("native SQLite benchmark failed: {error}");
        std::process::exit(1);
    }
}
