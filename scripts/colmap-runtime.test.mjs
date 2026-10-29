import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { archiveEntriesSafe, commitColmapRuntime, normalizeMacosRuntimePermissions, renameRuntimeDirectory } from "./setup-colmap-runtime.mjs";
import { assertHashPin, fileHash, filesUnder, isLockedColmapHelp, runtimeLock, verifyIntegrity, verifyColmap } from "./colmap-runtime.mjs";

function releaseFixture(t, platform = "linux") {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ooosplat-colmap-release-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const write = (name, contents) => {
    const target = path.join(root, ...name.split("/"));
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, contents);
  };
  const binary = `bin/${platform === "windows" ? "colmap.exe" : "colmap"}`;
  const shared = platform === "windows" ? "bin/runtime.dll" : platform === "macos" ? "lib/runtime.dylib" : "lib/runtime.so.1";
  const vocabulary = "lib/colmap/vocab_tree_faiss_flickr100K_words256K.bin";
  const models = ["lib/validation/model/cameras.txt", "lib/validation/model/images.txt", "lib/validation/model/points3D.txt"];
  write(binary, "colmap");
  write(shared, "runtime");
  write(vocabulary, "vocabulary");
  for (const model of models) write(model, "model");
  write("licenses/COLMAP/0-COPYING.txt", "BSD-3-Clause");

  const lock = structuredClone(runtimeLock);
  lock.offlineVocabulary = { path: vocabulary, sha256: fileHash(path.join(root, ...vocabulary.split("/"))), bytes: 10 };
  lock.platforms[platform] = { ...lock.platforms[platform], buildInfoSha256: "0".repeat(64) };
  const expected = lock.platforms[platform];
  const info = {
    schemaVersion: 1,
    platform,
    architecture: expected.architecture,
    sourceCommit: lock.sourceCommit,
    scriptCommit: lock.scriptCommit,
    repository: lock.repository,
    runtimeRevision: lock.runtimeRevision,
    colmapVersion: lock.colmapVersion,
    features: {
      ...Object.fromEntries(lock.disabledFeatures.map(feature => [feature, false])),
      cpuSift: true,
      ceresCpuBA: true,
      offlineLoopDetection: true,
      vocabTreeMatching: true,
      CUDA_ENABLED: expected.cuda,
      CASPAR_ENABLED: expected.caspar,
      CASPAR_USE_DOUBLE: false,
    },
    cudaRelease: expected.cuda ? lock.cudaRelease : null,
    gpuArchitectures: expected.cuda ? [...lock.gpuArchitectures] : [],
    offlineVocabulary: { ...lock.offlineVocabulary },
  };
  const runtimeFiles = [binary, shared, vocabulary, ...models];
  const inventory = { schemaVersion: 1, components: [{ name: "COLMAP", licenseFiles: ["licenses/COLMAP/0-COPYING.txt"], runtimeFiles }] };
  const writeMetadata = () => {
    write("BUILD-INFO.json", `${JSON.stringify(info)}\n`);
    lock.platforms[platform].buildInfoSha256 = fileHash(path.join(root, "BUILD-INFO.json"));
    write("BUNDLED-COMPONENTS.json", `${JSON.stringify(inventory)}\n`);
  };
  const reseal = () => {
    writeMetadata();
    write("SHA256SUMS", `${filesUnder(root).filter(file => path.basename(file) !== "SHA256SUMS").map(file => `${fileHash(file)}  ${path.relative(root, file).split(path.sep).join("/")}`).join("\n")}\n`);
    return fileHash(path.join(root, "SHA256SUMS"));
  };
  return { root, lock, info, inventory, write, reseal };
}

test("published runtime lock pins the requested immutable asset identities", () => {
  assert.equal(runtimeLock.repository, "ooolabdev/ooosplat-colmap");
  assert.equal(runtimeLock.releaseTag, "colmap-4.2.1-runtime.1");
  assert.equal(runtimeLock.sourceCommit, "bd1fcf654d2dd8fefa1466999c190a246f83f4b9");
  const expected = {
    windows: "6ea29a37f5ace05cf4003dd97143a5fe9e9d01b315f52405d0e38b8970b95d00",
    linux: "99d3e586b554069ff8fc524430924886608505ba8337751783682fae9eadf88d",
    macos: "fd7ac276a2b858e10f2bddd7febfac139c0d6f18b07d84c869c0363eb814530e",
  };
  for (const [platform, hash] of Object.entries(expected)) {
    assert.equal(runtimeLock.platforms[platform].archiveSha256, hash);
    assertHashPin(runtimeLock.platforms[platform].integritySha256, `${platform} inventory`);
    assert.match(runtimeLock.platforms[platform].sourceUrl, /github\.com\/ooolabdev\/ooosplat-colmap\/releases\/download\/colmap-4\.2\.1-runtime\.1\//);
  }
});

test("COLMAP identity accepts the locked full or standard short commit only", () => {
  assert.equal(isLockedColmapHelp(`COLMAP ${runtimeLock.colmapVersion} (Commit ${runtimeLock.sourceCommit})`), true);
  assert.equal(isLockedColmapHelp("COLMAP 4.2.1 (Commit bd1fcf6 on 2026-09-29 with CUDA)"), true);
  assert.equal(isLockedColmapHelp("COLMAP 4.2.1 (Commit bd1fcf60 on 2026-09-29 with CUDA)"), false);
  assert.equal(isLockedColmapHelp("COLMAP 4.1.0 (Commit bd1fcf6)"), false);
});

test("new Release metadata validates for all three platform policies", t => {
  for (const platform of ["windows", "linux", "macos"]) {
    const fixture = releaseFixture(t, platform);
    assert.doesNotThrow(() => verifyColmap(fixture.root, platform, fixture.reseal(), { run: false, requireRelease: true, releaseLock: fixture.lock }));
  }
});

test("identity, feature, platform and vocabulary drift are rejected", t => {
  for (const mutate of [
    fixture => { fixture.info.sourceCommit = "0".repeat(40); },
    fixture => { fixture.info.features.ONNX_ENABLED = true; },
    fixture => { fixture.info.architecture = "arm64"; },
    fixture => { fixture.info.offlineVocabulary.sha256 = "0".repeat(64); },
  ]) {
    const fixture = releaseFixture(t);
    mutate(fixture);
    assert.throws(() => verifyColmap(fixture.root, "linux", fixture.reseal(), { run: false, requireRelease: true, releaseLock: fixture.lock }), /identity|feature|architecture|vocabulary/i);
  }
});

test("inventory requires every dependency, license, model and vocabulary", t => {
  const fixture = releaseFixture(t);
  fixture.inventory.components[0].runtimeFiles = fixture.inventory.components[0].runtimeFiles.filter(file => file !== "lib/runtime.so.1");
  assert.throws(() => verifyColmap(fixture.root, "linux", fixture.reseal(), { run: false, requireRelease: true, releaseLock: fixture.lock }), /license inventory/);
});

test("changed, unlisted and traversal files are rejected", t => {
  const fixture = releaseFixture(t), pin = fixture.reseal();
  fixture.write("bin/colmap", "tampered");
  assert.throws(() => verifyIntegrity(fixture.root, pin), /hash mismatch/);
  const extra = releaseFixture(t), extraPin = extra.reseal();
  extra.write("bin/surprise.dll", "extra");
  assert.throws(() => verifyIntegrity(extra.root, extraPin), /Unverified runtime file/);
  extra.write("SHA256SUMS", `${"0".repeat(64)}  ../secret\n`);
  assert.throws(() => verifyIntegrity(extra.root, fileHash(path.join(extra.root, "SHA256SUMS"))), /Unsafe integrity path/);
});

test("archive listing accepts direct roots and rejects traversal or absolute paths", () => {
  assert.equal(archiveEntriesSafe("bin/\nbin/colmap\nBUILD-INFO.json\n"), true);
  for (const listing of ["", "../escape\n", "/absolute\n", "C:/absolute\n", "bin/../../escape\n"]) assert.equal(archiveEntriesSafe(listing), false);
});

test("macOS runtime permissions make read-only library copies replaceable", { skip: process.platform === "win32" }, t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ooosplat-colmap-permissions-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const library = path.join(root, "lib", "runtime.dylib");
  fs.mkdirSync(path.dirname(library));
  fs.writeFileSync(library, "runtime", { mode: 0o444 });
  fs.chmodSync(library, 0o444);

  normalizeMacosRuntimePermissions(root);

  assert.equal(fs.statSync(library).mode & 0o200, 0o200);
  fs.writeFileSync(library, "replacement");
  assert.equal(fs.readFileSync(library, "utf8"), "replacement");
});

function commitFixture(t, previous = true) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ooosplat-colmap-commit-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 }));
  const destination = path.join(root, "installed"), staged = path.join(root, "transaction", "runtime");
  if (previous) {
    fs.mkdirSync(destination, { recursive: true });
    fs.writeFileSync(path.join(destination, "identity"), "previous");
  }
  fs.mkdirSync(staged, { recursive: true });
  fs.writeFileSync(path.join(staged, "identity"), "candidate");
  return { root, destination, staged, backup: path.join(root, "transaction", "previous-runtime") };
}

test("failed installation verification restores the previous runtime", t => {
  const { destination, staged } = commitFixture(t);
  assert.throws(() => commitColmapRuntime(staged, destination, () => { throw new Error("candidate rejected"); }), /candidate rejected/);
  assert.equal(fs.readFileSync(path.join(destination, "identity"), "utf8"), "previous");
});

test("Windows runtime rename retries temporary sharing/access errors", () => {
  for (const code of ["EPERM", "EACCES", "EBUSY"]) {
    const waits = [], calls = [];
    renameRuntimeDirectory("staged", "installed", {
      platform: "win32",
      wait: milliseconds => waits.push(milliseconds),
      rename: (from, to) => {
        calls.push([from, to]);
        if (calls.length <= 2) throw Object.assign(new Error("temporarily locked"), { code });
      },
    });
    assert.deepEqual(waits, [100, 200]);
    assert.deepEqual(calls, Array.from({ length: 3 }, () => ["staged", "installed"]));
  }
});

test("permanent Windows locks exhaust a finite retry budget and retain the original error", () => {
  const failure = Object.assign(new Error("access denied"), { code: "EPERM" });
  const waits = [];
  let calls = 0;
  assert.throws(() => renameRuntimeDirectory("staged", "installed", {
    platform: "win32", wait: milliseconds => waits.push(milliseconds),
    rename: () => { calls++; throw failure; },
  }), error => error === failure);
  assert.equal(calls, 9);
  assert.equal(waits.reduce((sum, value) => sum + value, 0), 9100);
});

test("other errors and platforms fail immediately without retries", () => {
  for (const [platform, code] of [["win32", "ENOENT"], ["win32", "EXDEV"], ["win32", "ENOTEMPTY"], ["linux", "EPERM"], ["darwin", "EBUSY"]]) {
    const failure = Object.assign(new Error("rename failed"), { code });
    let calls = 0;
    assert.throws(() => renameRuntimeDirectory("staged", "installed", {
      platform, wait: () => assert.fail("Unexpected retry"),
      rename: () => { calls++; throw failure; },
    }), error => error === failure);
    assert.equal(calls, 1);
  }
});

test("Windows publication retries keep one verified installation transaction", t => {
  const { destination, staged, backup } = commitFixture(t);
  const failures = new Set([destination, staged]);
  let verified = 0;
  commitColmapRuntime(staged, destination, installed => {
    verified++;
    assert.equal(fs.readFileSync(path.join(installed, "identity"), "utf8"), "candidate");
  }, (from, to) => renameRuntimeDirectory(from, to, {
    platform: "win32", wait: () => {},
    rename: (source, target) => {
      if (failures.delete(source)) throw Object.assign(new Error("sharing violation"), { code: "EPERM" });
      fs.renameSync(source, target);
    },
  }));
  assert.equal(verified, 1);
  assert.equal(failures.size, 0);
  assert.equal(fs.readFileSync(path.join(backup, "identity"), "utf8"), "previous");
  assert.equal(fs.existsSync(staged), false);
});

test("permanent publication failure restores the previous runtime without verification", t => {
  const { destination, staged, backup } = commitFixture(t);
  const failure = Object.assign(new Error("publication denied"), { code: "EPERM" });
  assert.throws(() => commitColmapRuntime(staged, destination, () => assert.fail("Candidate was not installed"),
    (from, to) => renameRuntimeDirectory(from, to, {
      platform: "win32", wait: () => {},
      rename: (source, target) => {
        if (source === staged) throw failure;
        fs.renameSync(source, target);
      },
    })), error => error === failure);
  assert.equal(fs.readFileSync(path.join(destination, "identity"), "utf8"), "previous");
  assert.equal(fs.readFileSync(path.join(staged, "identity"), "utf8"), "candidate");
  assert.equal(fs.existsSync(backup), false);
});

test("rollback also retries temporary Windows locks", t => {
  const { destination, staged } = commitFixture(t);
  let rejected = false, retries = 0;
  assert.throws(() => commitColmapRuntime(staged, destination, () => {
    rejected = true;
    throw new Error("candidate rejected");
  }, (from, to) => renameRuntimeDirectory(from, to, {
    platform: "win32", wait: () => { retries++; },
    rename: (source, target) => {
      if (rejected) {
        rejected = false;
        throw Object.assign(new Error("sharing violation during rollback"), { code: "EBUSY" });
      }
      fs.renameSync(source, target);
    },
  })), /candidate rejected/);
  assert.equal(retries, 1);
  assert.equal(fs.readFileSync(path.join(destination, "identity"), "utf8"), "previous");
});

test("rollback failure preserves the previous runtime and both error causes", t => {
  const { destination, staged, backup } = commitFixture(t);
  const failure = new Error("candidate rejected"), rollbackFailure = new Error("restore denied");
  assert.throws(() => commitColmapRuntime(staged, destination, () => { throw failure; }, (from, to) => {
    if (from === backup) throw rollbackFailure;
    fs.renameSync(from, to);
  }), error => {
    assert.match(error.message, /candidate rejected.*restore denied.*recover the previous runtime/);
    assert.ok(error.message.includes(backup));
    assert.deepEqual(error.cause.errors, [failure, rollbackFailure]);
    return true;
  });
  assert.equal(fs.readFileSync(path.join(backup, "identity"), "utf8"), "previous");
});

test("Windows installation survives a real temporary file handle lock", { skip: process.platform !== "win32", timeout: 20_000 }, async t => {
  const { destination, staged } = commitFixture(t, false);
  const child = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `
    $ErrorActionPreference = 'Stop'
    $stream = [System.IO.File]::Open($env:OOOSPLAT_TEST_LOCK_FILE, 'Open', 'Read', 'Read')
    try {
      [Console]::WriteLine('locked')
      [Console]::Out.Flush()
      [Console]::ReadLine() | Out-Null
      [System.Threading.Thread]::Sleep(500)
    } finally { $stream.Dispose() }
  `], {
    windowsHide: true, timeout: 15_000,
    env: { ...process.env, OOOSPLAT_TEST_LOCK_FILE: path.join(staged, "identity") },
    stdio: ["pipe", "pipe", "pipe"],
  });
  t.after(() => { if (child.exitCode === null) child.kill(); });
  let diagnostics = "";
  child.stderr.on("data", data => { diagnostics += data; });
  const exited = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  const ready = new Promise(resolve => child.stdout.once("data", data => resolve(String(data))));
  const output = await Promise.race([ready, exited.then(result => { throw new Error(`Lock process exited before readiness: ${JSON.stringify(result)} ${diagnostics}`); })]);
  assert.match(output, /locked/);
  assert.throws(() => fs.renameSync(staged, destination), error => ["EPERM", "EACCES", "EBUSY"].includes(error.code));
  child.stdin.end("release\n");
  let verified = 0;
  commitColmapRuntime(staged, destination, installed => {
    verified++;
    assert.equal(fs.readFileSync(path.join(installed, "identity"), "utf8"), "candidate");
  });
  assert.equal(verified, 1);
  assert.deepEqual(await exited, { code: 0, signal: null }, diagnostics);
});
