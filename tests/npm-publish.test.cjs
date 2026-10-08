const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

test("resume partial npm publication and reject changed content or registry errors", { skip: process.platform === "win32" }, () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "bkmpw-publish-"));
  try {
    const version = "0.2.6";
    const suffixes = ["-linux-x64", "-linux-arm64", "-win32-x64", "-win32-arm64", "-darwin-x64", "-darwin-arm64", ""];
    for (const suffix of suffixes) {
      fs.writeFileSync(path.join(directory, `bro-know-my-packwiz${suffix}-${version}.tgz`), `archive ${suffix}`);
    }
    const statePath = path.join(directory, "registry.json");
    fs.writeFileSync(statePath, JSON.stringify({ entries: {}, published: [], failedOnce: false }));
    const fakeBin = path.join(directory, "bin");
    fs.mkdirSync(fakeBin);
    fs.writeFileSync(path.join(fakeBin, "npm"), `#!/usr/bin/env node
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const statePath = process.env.BKMPW_TEST_REGISTRY;
const state = JSON.parse(fs.readFileSync(statePath));
const [command, arg] = process.argv.slice(2);
if (command === 'view') {
  if (state.authError) { console.log(JSON.stringify({error:{code:'E401'}})); process.exit(1); }
  if (state.entries[arg]) { console.log(JSON.stringify(state.entries[arg])); process.exit(0); }
  console.log(JSON.stringify({error:{code:'E404'}})); process.exit(1);
}
if (!state.failedOnce && arg.includes('win32-x64')) {
  state.failedOnce = true; fs.writeFileSync(statePath, JSON.stringify(state)); process.exit(1);
}
const match = path.basename(arg).match(/^bro-know-my-(.+)-(\\d+\\.\\d+\\.\\d+)\\.tgz$/);
const spec = '@bro-know-my/' + match[1] + '@' + match[2];
state.entries[spec] = 'sha512-' + crypto.createHash('sha512').update(fs.readFileSync(arg)).digest('base64');
state.published.push(spec); fs.writeFileSync(statePath, JSON.stringify(state));
`, { mode: 0o755 });
    const publish = () => spawnSync(process.execPath, ["packages/npm/publish.mjs", directory, version], {
      encoding: "utf8", env: { ...process.env, PATH: fakeBin + path.delimiter + process.env.PATH, BKMPW_TEST_REGISTRY: statePath }
    });
    assert.notEqual(publish().status, 0);
    const retry = publish();
    assert.equal(retry.status, 0, retry.stderr);
    const state = JSON.parse(fs.readFileSync(statePath));
    assert.equal(state.published.length, 7);
    assert.equal(new Set(state.published).size, 7);
    assert.equal(state.published.at(-1), `@bro-know-my/packwiz@${version}`);
    state.authError = true;
    fs.writeFileSync(statePath, JSON.stringify(state));
    assert.match(publish().stderr, /Registry query failed/);
    state.authError = false;
    fs.writeFileSync(statePath, JSON.stringify(state));
    fs.appendFileSync(path.join(directory, `bro-know-my-packwiz-linux-x64-${version}.tgz`), "changed");
    assert.match(publish().stderr, /differs from this artifact/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
