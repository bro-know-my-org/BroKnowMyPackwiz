const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const os = require('node:os');
const { spawnSync } = require('node:child_process');

test('six native packages match the launcher, optional dependencies and host constraints', () => {
  const main = JSON.parse(fs.readFileSync('packages/npm/package.json', 'utf8'));
  const source = fs.readFileSync('packages/npm/bin/bkmpw.js', 'utf8');
  assert.equal(Object.keys(main.optionalDependencies).length, 6);
  for (const platform of ['linux', 'win32', 'darwin']) {
    for (const arch of ['x64', 'arm64']) {
      const suffix = `${platform}-${arch}`;
      const name = `@bro-know-my/packwiz-${suffix}`;
      const binary = platform === 'win32' ? 'bkmpw.exe' : 'bkmpw';
      const metadata = JSON.parse(fs.readFileSync(`packages/npm-platforms/${suffix}/package.json`, 'utf8'));
      assert.equal(metadata.name, name);
      assert.deepEqual(metadata.os, [platform]);
      assert.deepEqual(metadata.cpu, [arch]);
      assert.ok(metadata.files.includes(binary));
      assert.equal(main.optionalDependencies[name], metadata.version);
      let spawned = false, status;
      const mockRequire = module => {
        if (module === 'node:path') return path;
        assert.equal(module, 'node:child_process');
        return { spawnSync(executable, args, options) {
          spawned = true;
          assert.equal(executable, path.join('native', binary));
          assert.deepEqual(Array.from(args), ['--version', 'a path with spaces']);
          assert.equal(options.env.BKMPW_MANAGED_BY_NPM, '1');
          return { status: 7 };
        } };
      };
      mockRequire.resolve = module => {
        assert.equal(module, `${name}/package.json`);
        return path.join('native', 'package.json');
      };
      vm.runInNewContext(source, { require: mockRequire, console, process: {
        platform, arch, env: {}, argv: ['node', 'bkmpw.js', '--version', 'a path with spaces'],
        exit(code) { status = code; }
      } });
      assert.ok(spawned);
      assert.equal(status, 7);
    }
  }
});

test('native npm launcher executes the real release binary', { skip: !process.env.BKMPW_TEST_BINARY }, () => {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'bkmpw-native-smoke-'));
  try {
    const name = `@bro-know-my/packwiz-${process.platform}-${process.arch}`;
    const native = path.join(temporary, 'node_modules', name);
    fs.mkdirSync(native, { recursive: true });
    fs.writeFileSync(path.join(native, 'package.json'), JSON.stringify({ name }));
    const executable = path.join(native, process.platform === 'win32' ? 'bkmpw.exe' : 'bkmpw');
    fs.copyFileSync(process.env.BKMPW_TEST_BINARY, executable);
    fs.chmodSync(executable, 0o755);
    const launcher = path.join(temporary, 'launcher.js');
    fs.copyFileSync('packages/npm/bin/bkmpw.js', launcher);
    const version = fs.readFileSync('Cargo.toml', 'utf8').match(/^version\s*=\s*"([^"]+)"/m)[1];
    const env = { ...process.env };
    delete env.BKMPW_BINARY_PATH;
    const result = spawnSync(process.execPath, [launcher, '--version'], { encoding: 'utf8', env, timeout: 15000 });
    assert.ifError(result.error);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout.trim(), `bkmpw ${version}`);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
});
