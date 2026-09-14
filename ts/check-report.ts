/**
 * Validates the JSON report contract.
 *
 * The text report is for humans; this JSON shape is what a CI gate would read,
 * so it needs a consumer outside Rust that breaks if the schema drifts. Run it
 * with `pnpm verify:report` (Node >= 22.6 strips the types natively; there is
 * no build step and no dependencies).
 */

import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

type Difference = {
  kind: string;
  account?: string;
  field?: string;
  v1?: unknown;
  v2?: unknown;
};

type Diff = {
  fixture_id: string;
  category: string;
  scenario: string;
  differences: Difference[];
};

type Report = {
  program_id: string;
  v1_artifact: string;
  v2_artifact: string;
  summary: {
    fixtures_tested: number;
    outcome_identical: number;
    changed: number;
    critical: number;
    compute: { fixtures_with_delta: number; above_regression_threshold: number };
  };
  diffs: Diff[];
};

const failures: string[] = [];
function check(condition: boolean, message: string): void {
  if (!condition) failures.push(message);
}

console.log("running: eplyx compare --format json");
const raw = execFileSync(
  "cargo",
  ["run", "-q", "-p", "eplyx-engine", "--", "compare", "--format", "json"],
  { cwd: repoRoot, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
);

let report: Report;
try {
  report = JSON.parse(raw) as Report;
} catch (error) {
  console.error("report was not valid JSON:", error);
  process.exit(1);
}

const { summary } = report;

check(typeof report.program_id === "string" && report.program_id.length > 0, "program_id missing");
check(summary.fixtures_tested >= 100, `corpus too small: ${summary.fixtures_tested}`);
check(
  summary.outcome_identical + summary.changed === summary.fixtures_tested,
  "fixture counts do not add up",
);
check(summary.critical > 0, "expected at least one critical regression");
check(
  summary.outcome_identical > summary.changed,
  "a regression affecting most of the corpus would not be subtle",
);
check(report.diffs.length === summary.fixtures_tested, "diffs array length disagrees with summary");

const flagship = report.diffs.find((d) => d.fixture_id === "boundary-position-017");
check(flagship !== undefined, "flagship fixture boundary-position-017 missing");

if (flagship) {
  const kinds = new Set(flagship.differences.map((d) => d.kind));
  check(
    kinds.has("liquidation_status_changed"),
    "flagship fixture does not report a liquidation status change",
  );
  const flip = flagship.differences.find((d) => d.kind === "liquidation_status_changed");
  check(flip?.v1 === false && flip?.v2 === true, "flagship flip direction is wrong");
}

for (const diff of report.diffs) {
  check(typeof diff.fixture_id === "string" && diff.fixture_id.length > 0, "diff without an id");
  for (const difference of diff.differences) {
    check(
      typeof difference.kind === "string" && difference.kind.length > 0,
      `${diff.fixture_id}: difference without a kind tag`,
    );
  }
}

if (failures.length > 0) {
  console.error(`\n${failures.length} contract violation(s):`);
  for (const failure of failures) console.error(`  - ${failure}`);
  process.exit(1);
}

console.log(
  [
    "",
    "report contract OK",
    `  fixtures tested:    ${summary.fixtures_tested}`,
    `  outcome identical:  ${summary.outcome_identical}`,
    `  outcome changed:    ${summary.changed}`,
    `  critical:           ${summary.critical}`,
    `  compute deltas:     ${summary.compute.fixtures_with_delta} (${summary.compute.above_regression_threshold} above threshold)`,
  ].join("\n"),
);
