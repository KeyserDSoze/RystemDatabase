import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const workflow = readFileSync(
  fileURLToPath(new URL("../../.github/workflows/ci.yml", import.meta.url)), "utf8"
);
const matrix = "os: [ubuntu-latest, macos-latest, windows-latest]";

function jobBody(name, nextName) {
  const start = workflow.indexOf("  " + name + ":\n");
  const end = workflow.indexOf("\n  " + nextName + ":\n", start);
  assert.ok(start !== -1 && end > start, "CI jobs must be stable and explicit");
  return workflow.slice(start, end);
}

test("TDD RED #48: Node live Rust RPC is tested on Linux macOS and Windows", () => {
  const job = jobBody("npm", "dotnet");
  assert.ok(job.includes(matrix), "Node interop requires a genuine OS matrix");
  assert.match(job, /RYSTEMDB_BINARY:/);
  assert.match(job, /runner\.os == 'Windows'.*\.exe/,
    "Windows Node must launch an actual .exe built by Cargo");
  assert.match(job, /cargo build --locked -p rystemdb-cli/);
  assert.match(job, /working-directory: src\/libraries\/typescript\/rystemdb-client/);
});

test("TDD RED #48: .NET live Rust RPC is tested on Linux macOS and Windows", () => {
  const job = jobBody("dotnet", "docker");
  assert.ok(job.includes(matrix), ".NET live interop requires a genuine OS matrix");
  assert.match(job, /runner\.os == 'Windows'.*\.exe/,
    "Windows .NET tests must receive an actual Rust .exe path");
  assert.match(job, /cargo build --locked -p rystemdb-cli/);
  assert.match(job, /RystemDatabase\.Client\.Tests\.csproj/);
  assert.match(job, /10\.0\.x/, "Pin required .NET major");
});

test("TDD RED #48: SDK jobs run actual live tests, never silently skip them", () => {
  const npm = jobBody("npm", "dotnet");
  const dotnet = jobBody("dotnet", "docker");
  assert.doesNotMatch(npm, /continue-on-error:\s*true/);
  assert.doesNotMatch(dotnet, /continue-on-error:\s*true/);
  assert.match(npm, /node-version: '22'/);
  assert.match(dotnet, /dotnet run --project/);
});
