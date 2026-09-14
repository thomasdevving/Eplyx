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

/** Monetary values are decimal strings, never JSON numbers: parsing them as
 *  doubles would reintroduce the precision loss the engine avoids. */
type UsdString = string;

type CapitalGroup = {
  positions: number;
  collateral_value_usd: UsdString;
  debt_value_usd: UsdString;
};

type Economics = {
  positions_valued: number;
  total_collateral_value_usd: UsdString;
  total_debt_value_usd: UsdString;
  total_net_value_usd: UsdString;
  affected: CapitalGroup;
  critical: CapitalGroup;
  newly_liquidatable: CapitalGroup;
  by_consequence: Record<string, CapitalGroup>;
};

type FixtureEconomics = {
  fixture_id: string;
  baseline: {
    collateral_value_usd: UsdString;
    debt_value_usd: UsdString;
    net_value_usd: UsdString;
    collateral_decimals: number;
    debt_decimals: number;
    liquidatable: boolean;
  };
  affected: boolean;
  critical: boolean;
  consequences: string[];
};

type NumericRange = {
  field: string;
  metric: string;
  min: number;
  max: number;
  min_display: string;
  max_display: string;
  informative: boolean;
};

type MinimizedCase = {
  derived_from: string;
  collateral_lamports: number;
  debt_micro_usd: number;
  collateral_value_usd: UsdString;
  debt_value_usd: UsdString;
  v1_liquidatable: boolean;
  v2_liquidatable: boolean;
  v1_outcome: string;
  v2_outcome: string;
  probes: number;
  reductions: number;
};

type RegressionCluster = {
  id: string;
  consequences: string[];
  action: string;
  difference_kinds: string[];
  critical: boolean;
  fixture_ids: string[];
  collateral_value_usd: UsdString;
  debt_value_usd: UsdString;
  common_conditions: Array<{ field: string; value: string }>;
  ranges: NumericRange[];
  representative_fixture_id: string;
  minimized_counterexample: MinimizedCase | null;
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
    compute: {
      fixtures_with_delta: number;
      above_regression_threshold: number;
      min_pct_bps: number;
      max_pct_bps: number;
    };
  };
  economics: Economics;
  fixture_economics: FixtureEconomics[];
  clusters: RegressionCluster[];
  diffs: Diff[];
};

/** Parse a fixed-point USD string into integer micro-USD, without ever
 *  producing a fractional double. */
function toMicroUsd(value: UsdString): bigint {
  const match = /^(-?)(\d+)\.(\d{6})$/.exec(value);
  if (match === null) throw new Error(`malformed USD value: ${JSON.stringify(value)}`);
  const [, sign, whole, fraction] = match;
  const micro = BigInt(whole) * 1_000_000n + BigInt(fraction);
  return sign === "-" ? -micro : micro;
}

function formatUsd(value: UsdString): string {
  const dollars = toMicroUsd(value) / 1_000_000n;
  return `$${dollars.toLocaleString("en-US")}`;
}

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

// ---- economic impact contract --------------------------------------------

const econ = report.economics;
check(econ !== undefined, "economics summary missing");

if (econ) {
  check(
    econ.positions_valued === summary.fixtures_tested,
    "positions_valued disagrees with fixtures_tested",
  );
  check(
    report.fixture_economics.length === econ.positions_valued,
    "fixture_economics length disagrees with positions_valued",
  );

  // Every monetary field must parse as exact fixed point.
  const monetary: Array<[string, UsdString]> = [
    ["total_collateral_value_usd", econ.total_collateral_value_usd],
    ["total_debt_value_usd", econ.total_debt_value_usd],
    ["total_net_value_usd", econ.total_net_value_usd],
    ["affected.collateral_value_usd", econ.affected.collateral_value_usd],
    ["affected.debt_value_usd", econ.affected.debt_value_usd],
    ["newly_liquidatable.collateral_value_usd", econ.newly_liquidatable.collateral_value_usd],
    ["newly_liquidatable.debt_value_usd", econ.newly_liquidatable.debt_value_usd],
  ];
  for (const [name, value] of monetary) {
    check(typeof value === "string", `${name} must be a string, not a JSON number`);
    try {
      toMicroUsd(value);
    } catch (error) {
      failures.push(`${name}: ${(error as Error).message}`);
    }
  }

  check(
    toMicroUsd(econ.total_net_value_usd) ===
      toMicroUsd(econ.total_collateral_value_usd) - toMicroUsd(econ.total_debt_value_usd),
    "net value does not equal collateral minus debt",
  );

  // Aggregates must equal the sum of their member positions, computed here
  // independently of the engine.
  const affectedCollateral = report.fixture_economics
    .filter((e) => e.affected)
    .reduce((total, e) => total + toMicroUsd(e.baseline.collateral_value_usd), 0n);
  check(
    affectedCollateral === toMicroUsd(econ.affected.collateral_value_usd),
    "affected collateral does not equal the sum of affected positions",
  );

  const totalCollateral = report.fixture_economics.reduce(
    (total, e) => total + toMicroUsd(e.baseline.collateral_value_usd),
    0n,
  );
  check(
    totalCollateral === toMicroUsd(econ.total_collateral_value_usd),
    "total collateral does not equal the sum of all positions",
  );

  // Affected capital must be a strict subset: compute-only differences never
  // count, and the corpus has many of those.
  check(
    toMicroUsd(econ.affected.collateral_value_usd) <
      toMicroUsd(econ.total_collateral_value_usd),
    "affected collateral should be strictly less than total collateral",
  );
  check(
    econ.affected.positions === summary.changed,
    "affected positions should match the changed count",
  );
  check(
    econ.newly_liquidatable.positions > 0,
    "expected at least one newly liquidatable position",
  );

  const newlyLiquidatable = report.fixture_economics.filter((e) =>
    e.consequences.includes("newly_liquidatable"),
  );
  check(
    newlyLiquidatable.length === econ.newly_liquidatable.positions,
    "newly liquidatable count disagrees with per-fixture consequences",
  );

  // Every affected position carries at least one consequence, and vice versa.
  for (const entry of report.fixture_economics) {
    check(
      entry.affected === (entry.consequences.length > 0),
      `${entry.fixture_id}: affected flag disagrees with consequences`,
    );
  }
}

if (flagship) {
  const kinds = new Set(flagship.differences.map((d) => d.kind));
  check(
    kinds.has("liquidation_status_changed"),
    "flagship fixture does not report a liquidation status change",
  );
  const flip = flagship.differences.find((d) => d.kind === "liquidation_status_changed");
  check(flip?.v1 === false && flip?.v2 === true, "flagship flip direction is wrong");
}

// ---- regression clustering contract ---------------------------------------

const clusters = report.clusters;
check(Array.isArray(clusters) && clusters.length > 0, "clusters missing");

if (Array.isArray(clusters)) {
  const ids = new Set<string>();
  const clustered: string[] = [];

  for (const cluster of clusters) {
    check(!ids.has(cluster.id), `duplicate cluster id: ${cluster.id}`);
    ids.add(cluster.id);

    check(cluster.fixture_ids.length > 0, `${cluster.id}: empty cluster`);
    check(
      cluster.fixture_ids.includes(cluster.representative_fixture_id),
      `${cluster.id}: representative is not a member`,
    );
    check(cluster.consequences.length > 0, `${cluster.id}: no consequence`);
    check(
      typeof cluster.collateral_value_usd === "string",
      `${cluster.id}: collateral must be a decimal string`,
    );

    // Cluster capital must equal the sum of its members, recomputed here.
    const memberTotal = cluster.fixture_ids.reduce((total, id) => {
      const entry = report.fixture_economics.find((e) => e.fixture_id === id);
      return entry === undefined ? total : total + toMicroUsd(entry.baseline.collateral_value_usd);
    }, 0n);
    check(
      memberTotal === toMicroUsd(cluster.collateral_value_usd),
      `${cluster.id}: collateral does not equal the sum of its members`,
    );

    for (const range of cluster.ranges) {
      check(range.min <= range.max, `${cluster.id}: range ${range.field} is inverted`);
    }

    // Minimization only runs for critical clusters, and only when enabled.
    const minimized = cluster.minimized_counterexample;
    if (minimized !== null && minimized !== undefined) {
      check(cluster.critical, `${cluster.id}: non-critical cluster was minimized`);
      check(
        minimized.derived_from === cluster.representative_fixture_id,
        `${cluster.id}: minimized case derived from a non-representative fixture`,
      );
      check(minimized.reductions > 0, `${cluster.id}: minimized case reduced nothing`);
      check(
        minimized.probes >= minimized.reductions,
        `${cluster.id}: fewer probes than reductions`,
      );
      check(
        typeof minimized.collateral_value_usd === "string",
        `${cluster.id}: minimized values must be decimal strings`,
      );
    }

    clustered.push(...cluster.fixture_ids);
  }

  // Clusters must partition the changed fixtures: no overlap, no omission.
  check(
    new Set(clustered).size === clustered.length,
    "a fixture appears in more than one cluster",
  );
  check(
    clustered.length === summary.changed,
    `clusters cover ${clustered.length} fixtures but ${summary.changed} changed`,
  );

  const criticalClusters = clusters.filter((c) => c.critical);
  check(criticalClusters.length > 0, "expected at least one critical cluster");
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
    `  fixtures tested:        ${summary.fixtures_tested}`,
    `  outcome identical:      ${summary.outcome_identical}`,
    `  outcome changed:        ${summary.changed}`,
    `  critical:               ${summary.critical}`,
    `  compute deltas:         ${summary.compute.fixtures_with_delta} (${summary.compute.above_regression_threshold} above threshold)`,
    "",
    `  collateral represented: ${formatUsd(econ.total_collateral_value_usd)}`,
    `  debt represented:       ${formatUsd(econ.total_debt_value_usd)}`,
    `  affected collateral:    ${formatUsd(econ.affected.collateral_value_usd)} across ${econ.affected.positions} positions`,
    `  newly liquidatable:     ${formatUsd(econ.newly_liquidatable.collateral_value_usd)} across ${econ.newly_liquidatable.positions} positions`,
    "",
    `  regression clusters:    ${clusters.length} (${clusters.filter((c) => c.critical).length} critical)`,
    ...clusters.map(
      (c) =>
        `    ${c.id.padEnd(34)} ${String(c.fixture_ids.length).padStart(3)} fixtures` +
        (c.minimized_counterexample
          ? `  minimized in ${c.minimized_counterexample.probes} probes`
          : ""),
    ),
  ].join("\n"),
);
