#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = 'raihan0x1/0x1-ai-memory';
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(readFileSync(path.join(__dirname, '..', 'package.json'), 'utf8'));
const VERSION = (process.env.OX1_MEMORY_VERSION || pkg.version).replace(/^v/, '');
const IS_WIN = process.platform === 'win32';
const BIN_NAME = IS_WIN ? '0x1-memory.exe' : '0x1-memory';

const ASSET_MAP = {
  'darwin:arm64': '0x1-memory-darwin-arm64.tar.gz',
  'linux:x64': '0x1-memory-linux-x64.tar.gz',
  'win32:x64': '0x1-memory-windows-x64.zip',
};

function resolveAsset() {
  const key = `${process.platform}:${process.arch}`;
  const asset = ASSET_MAP[key];
  if (!asset) {
    console.error(`No prebuilt 0x1-memory binary for ${process.platform}/${process.arch} yet.`);
    console.error('Build from source:');
    console.error(
      `  git clone https://github.com/${REPO}.git && cd 0x1-ai-memory && (cd ui && pnpm install && pnpm build) && cargo build --release`,
    );
    process.exit(1);
  }
  return asset;
}

function stableBinPaths() {
  const dataDir = process.env.OX1_MEMORY_DATA_DIR || path.join(os.homedir(), '.0x1-memory');
  const binDir = path.join(dataDir, 'bin');
  return {
    binDir,
    binPath: path.join(binDir, BIN_NAME),
    stampPath: path.join(binDir, '.version'),
  };
}

async function fetchBuffer(url) {
  const res = await fetch(url, { redirect: 'follow' });
  if (!res.ok) {
    throw new Error(`HTTP ${res.status} downloading ${url}`);
  }
  return Buffer.from(await res.arrayBuffer());
}

function parseChecksum(sumsText, asset) {
  for (const line of sumsText.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const match = trimmed.match(/^([a-fA-F0-9]{64})\s+\*?(.+)$/);
    if (match && match[2] === asset) {
      return match[1].toLowerCase();
    }
  }
  return null;
}

function resolveSystemBin(candidates) {
  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }
  throw new Error(`Required system tool not found in standard paths: ${candidates.join(', ')}`);
}

function extractArchive(archivePath, destDir) {
  if (archivePath.endsWith('.tar.gz')) {
    const tarBin = resolveSystemBin(
      IS_WIN
        ? [path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe')]
        : ['/usr/bin/tar', '/bin/tar'],
    );
    const res = spawnSync(tarBin, ['-xzf', archivePath, '-C', destDir], { stdio: 'inherit' });
    if (res.status !== 0) {
      throw new Error(`Failed to extract ${path.basename(archivePath)} with tar`);
    }
    return;
  }
  if (IS_WIN) {
    const psBin = resolveSystemBin([
      path.join(
        process.env.SystemRoot || 'C:\\Windows',
        'System32',
        'WindowsPowerShell',
        'v1.0',
        'powershell.exe',
      ),
    ]);
    const res = spawnSync(
      psBin,
      [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        `Expand-Archive -Force -LiteralPath '${archivePath.replace(/'/g, "''")}' -DestinationPath '${destDir.replace(/'/g, "''")}'`,
      ],
      { stdio: 'inherit' },
    );
    if (res.status !== 0) {
      throw new Error(`Failed to extract ${path.basename(archivePath)} with PowerShell`);
    }
    return;
  }
  const unzipBin = resolveSystemBin(['/usr/bin/unzip', '/bin/unzip']);
  const res = spawnSync(unzipBin, ['-o', archivePath, '-d', destDir], { stdio: 'inherit' });
  if (res.status !== 0) {
    throw new Error(`Failed to extract ${path.basename(archivePath)}`);
  }
}

async function ensureBinary() {
  if (process.env.OX1_MEMORY_BIN && existsSync(process.env.OX1_MEMORY_BIN)) {
    return process.env.OX1_MEMORY_BIN;
  }

  const { binDir, binPath, stampPath } = stableBinPaths();
  if (existsSync(binPath) && existsSync(stampPath)) {
    const installedVer = readFileSync(stampPath, 'utf8').trim();
    if (installedVer === VERSION) {
      return binPath;
    }
  }

  const asset = resolveAsset();
  const baseUrl = `https://github.com/${REPO}/releases/download/v${VERSION}`;
  console.error(`Downloading 0x1-memory v${VERSION} (${asset})...`);

  const tmpDir = mkdtempSync(path.join(os.tmpdir(), 'ox1-memory-'));
  try {
    const archiveBuf = await fetchBuffer(`${baseUrl}/${asset}`);

    if (process.env.OX1_SKIP_CHECKSUM !== '1') {
      const sumsText = (await fetchBuffer(`${baseUrl}/SHA256SUMS.txt`)).toString('utf8');
      const expected = parseChecksum(sumsText, asset);
      if (!expected) {
        throw new Error(`Checksum entry missing for ${asset} in SHA256SUMS.txt (aborting)`);
      }
      const actual = createHash('sha256').update(archiveBuf).digest('hex');
      if (actual !== expected) {
        throw new Error(`Checksum mismatch for ${asset}! Expected ${expected}, got ${actual}`);
      }
    }

    const archivePath = path.join(tmpDir, asset);
    writeFileSync(archivePath, archiveBuf);
    extractArchive(archivePath, tmpDir);

    const extracted = path.join(tmpDir, BIN_NAME);
    if (!existsSync(extracted)) {
      throw new Error(`Extracted archive did not contain ${BIN_NAME}`);
    }

    mkdirSync(binDir, { recursive: true });
    const wasUpgrade = existsSync(binPath);
    const stagedPath = path.join(binDir, `${BIN_NAME}.tmp-${process.pid}`);
    writeFileSync(stagedPath, readFileSync(extracted));
    if (!IS_WIN) {
      chmodSync(stagedPath, 0o755);
    }
    renameSync(stagedPath, binPath);
    writeFileSync(stampPath, `${VERSION}\n`);
    console.error(`Installed native Rust binary to ${binPath}`);
    const subcmd = process.argv[2];
    if (wasUpgrade && subcmd && subcmd !== 'start' && subcmd !== 'restart') {
      console.error(
        `Upgraded binary to v${VERSION}. Run '0x1-memory restart' if a Daemon is already running.`,
      );
    }
    return binPath;
  } finally {
    rmSync(tmpDir, { recursive: true, force: true });
  }
}

async function main() {
  const binPath = await ensureBinary();
  const child = spawn(binPath, process.argv.slice(2), { stdio: 'inherit' });

  for (const sig of ['SIGINT', 'SIGTERM']) {
    process.on(sig, () => {
      if (!child.killed) {
        child.kill(sig);
      }
    });
  }

  child.on('exit', (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    } else {
      process.exit(code ?? 0);
    }
  });
}

main().catch((err) => {
  console.error(`0x1-ai-memory launcher error: ${err.message}`);
  process.exit(1);
});
