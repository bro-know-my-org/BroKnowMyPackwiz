#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const path = require("node:path");

const packages = {
  "darwin arm64": ["@bro-know-my/packwiz-darwin-arm64", "bkmpw"],
  "darwin x64": ["@bro-know-my/packwiz-darwin-x64", "bkmpw"],
  "linux x64": ["@bro-know-my/packwiz-linux-x64", "bkmpw"],
  "win32 x64": ["@bro-know-my/packwiz-win32-x64", "bkmpw.exe"]
};

function resolveBinary() {
  if (process.env.BKMPW_BINARY_PATH) {
    return process.env.BKMPW_BINARY_PATH;
  }

  const entry = packages[`${process.platform} ${process.arch}`];
  if (!entry) {
    throw new Error(`Unsupported platform: ${process.platform} ${process.arch}`);
  }

  const [packageName, binaryName] = entry;
  let packageJson;
  try {
    packageJson = require.resolve(`${packageName}/package.json`);
  } catch {
    throw new Error(
      `Missing native package ${packageName}. Reinstall @bro-know-my/packwiz for this platform.`
    );
  }
  return path.join(path.dirname(packageJson), binaryName);
}

let binary;
try {
  binary = resolveBinary();
} catch (error) {
  console.error(`bkmpw: ${error.message}`);
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), {
  stdio: "inherit",
  env: {
    ...process.env,
    BKMPW_MANAGED_BY_NPM: "1"
  }
});
if (result.error) {
  console.error(`bkmpw: failed to run ${binary}: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 1);
