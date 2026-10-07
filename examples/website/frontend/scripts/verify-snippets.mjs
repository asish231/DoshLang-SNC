// Verifies every UI snippet: compiles with ./snc from the repo root
// (same module resolution as the website sandbox) and runs it.
// Usage: node scripts/verify-snippets.mjs
import { execFileSync } from 'node:child_process';
import { readdirSync, copyFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..', '..', '..');
const dir = join(root, 'examples', 'website', 'frontend', 'snippets');
// filedemo.sn writes a file when run -> compile only, like the guide UI treats it.
const COMPILE_ONLY = new Set(['filedemo.sn']);

const files = readdirSync(dir).filter((f) => f.endsWith('.sn')).sort();
let fail = 0;
for (const f of files) {
  const tmp = join(root, 'examples', 'website', '.verify_' + f);
  const bin = tmp + '.bin';
  try {
    copyFileSync(join(dir, f), tmp);
    execFileSync(join(root, 'snc'), [tmp, '-o', bin], { cwd: root, stdio: 'pipe' });
    if (!COMPILE_ONLY.has(f)) {
      execFileSync(bin, { cwd: root, stdio: 'pipe', timeout: 15000 });
    }
    console.log('OK ' + f);
  } catch (e) {
    fail += 1;
    const out = (e.stdout || '') + '' + (e.stderr || '') + (e.message || '');
    console.log('FAIL ' + f + '\n' + String(out).split('\n').slice(0, 6).join('\n'));
  } finally {
    rmSync(tmp, { force: true });
    rmSync(bin, { force: true });
  }
}
if (fail > 0) {
  console.log(fail + ' snippet(s) failed');
  process.exit(1);
}
console.log('all ' + files.length + ' snippets verified');
