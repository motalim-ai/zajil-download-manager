// Every npm package that goes into the app must have a license compatible
// with GPL-3.0 (AGENTS.md §3; cargo-deny does not look at npm). Build and
// test tools are listed too, with their own allowed list.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

export const SHIPPED = new Set(["MIT", "ISC", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "OFL-1.1", "0BSD", "Unlicense", "CC0-1.0", "Zlib"]);
export const TOOLS = new Set([...SHIPPED, "MPL-2.0", "BlueOak-1.0.0", "CC-BY-4.0", "Python-2.0", "Unicode-3.0"]);

/// Whether an SPDX expression is allowed by `list`: OR needs one side, AND
/// both, `WITH <exception>` only widens what the license allows, and
/// parentheses group (review of #45: they were dropped, so
/// `(MIT OR GPL-2.0-only) AND SSPL-1.0` passed). Anything not understood is
/// refused.
export function allowed(expr, list) {
  const tokens = String(expr).match(/\(|\)|[^\s()]+/g) ?? [];
  let at = 0;
  const fail = () => {
    throw new Error(`not an SPDX expression: ${expr}`);
  };
  const primary = () => {
    const token = tokens[at++];
    if (token === "(") {
      const value = or();
      if (tokens[at++] !== ")") fail();
      return value;
    }
    if (!token || [")", "AND", "OR", "WITH"].includes(token)) fail();
    if (tokens[at] === "WITH") {
      at++;
      if (!tokens[at++]) fail();
    }
    return list.has(token);
  };
  const and = () => {
    let value = primary();
    while (tokens[at] === "AND") {
      at++;
      value = primary() && value;
    }
    return value;
  };
  const or = () => {
    let value = and();
    while (tokens[at] === "OR") {
      at++;
      value = and() || value;
    }
    return value;
  };
  try {
    const value = or();
    return at === tokens.length && value;
  } catch {
    return false;
  }
}

function walk(tree, dev, seen, bad) {
  for (const [name, node] of Object.entries(tree.dependencies ?? {})) {
    if (!node.path || seen.has(node.path)) continue;
    seen.add(node.path);
    const pkg = JSON.parse(readFileSync(join(node.path, "package.json"), "utf8"));
    const license = typeof pkg.license === "string" ? pkg.license : pkg.license?.type ?? "UNKNOWN";
    if (!allowed(license, dev ? TOOLS : SHIPPED)) bad.push(`${name}@${pkg.version}: ${license}${dev ? " (tool)" : ""}`);
    walk(node, dev, seen, bad);
  }
}

// Run as a script (not when the tests import `allowed`).
if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) main();

function main() {
  const run = (args) => JSON.parse(execFileSync("npm", args, { encoding: "utf8", shell: process.platform === "win32" }));
  const bad = [];
  walk(run(["ls", "--all", "--json", "--long", "--omit=dev"]), false, new Set(), bad);
  const shippedPaths = new Set();
  walk(run(["ls", "--all", "--json", "--long"]), true, shippedPaths, bad);
  if (bad.length) {
    console.error("licenses not allowed:\n" + [...new Set(bad)].join("\n"));
    process.exit(1);
  }
  console.log("npm licenses: ok");
}
