import React from 'react';
import { api, cleanOutput } from '../sn.jsx';
import { SNIPPETS, PRESETS } from '../data/snippets.js';

export default function Playground({ presetId }) {
  const startId = presetId && SNIPPETS[presetId] ? presetId : 'hello';
  const [code, setCode] = React.useState(SNIPPETS[startId]);
  const [preset, setPreset] = React.useState(startId);
  const [running, setRunning] = React.useState(false);
  const [result, setResult] = React.useState(null);

  function pick(id) {
    setPreset(id);
    setCode(SNIPPETS[id] || '');
    setResult(null);
  }

  async function run() {
    setRunning(true);
    setResult(null);
    try {
      const r = await api.run(code);
      setResult({ ok: r.exit_code === 0, out: cleanOutput(r.output), ms: r.duration_ms, code: r.exit_code });
    } catch (e) {
      const msg = String((e && e.message) || e);
      setResult({
        ok: false,
        out: msg.includes('Failed to fetch')
          ? 'Cannot reach the SNlang server.\n\nStart it first:\n  ./snc examples/website/server.sn -o website_server\n  ./website_server   # serves this site on :8090'
          : msg,
      });
    } finally {
      setRunning(false);
    }
  }

  return (
    <div>
      <div className="playbar">
        <select value={preset} onChange={(e) => pick(e.target.value)}>
          {PRESETS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.title}
            </option>
          ))}
          {!PRESETS.some((p) => p.id === preset) && <option value={preset}>Guide snippet: {preset}</option>}
        </select>
        <button className="btn primary" onClick={run} disabled={running}>
          {running ? 'Compiling & running…' : 'Run with snc'}
        </button>
        <span style={{ color: 'var(--faint)', fontSize: 13 }}>compiled to native code on the server, then executed</span>
      </div>
      <div className="play">
        <textarea value={code} onChange={(e) => setCode(e.target.value)} spellCheck={false} />
        <div className="pane">
          <header>
            <span className={result ? (result.ok ? 'ok' : 'bad') : ''} style={{ fontWeight: 700, color: result ? (result.ok ? 'var(--accent)' : 'var(--red)') : 'var(--muted)' }}>
              {running ? 'running…' : result ? (result.ok ? 'exit 0' : 'failed') : 'output'}
            </span>
            {result && result.ms != null && <span style={{ color: 'var(--faint)', fontWeight: 400 }}>exit_code={result.code} · {result.ms} ms</span>}
          </header>
          <div className="body">{result ? result.out || '(no output)' : 'Press Run to compile and execute.'}</div>
        </div>
      </div>
    </div>
  );
}
