import React from 'react';
import { api, cleanOutput, highlight, nav } from '../sn.jsx';
import { SNIPPETS } from '../data/snippets.js';

export default function CodeBlock({ code, lang = 'sn', title, runnable = false, output, snippetId, note }) {
  const [running, setRunning] = React.useState(false);
  const [result, setResult] = React.useState(null);
  const [copied, setCopied] = React.useState(false);

  async function run() {
    setRunning(true);
    setResult(null);
    try {
      const r = await api.run(code);
      setResult({ ok: r.exit_code === 0, out: cleanOutput(r.output), ms: r.duration_ms, code: r.exit_code });
    } catch (e) {
      const msg = String(e.message || e);
      setResult({ ok: false, out: msg.includes('Failed to fetch') ? 'Cannot reach the SNlang server. Start it (see Get Started) and reload.' : msg });
    } finally {
      setRunning(false);
    }
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
    } catch {
      const ta = document.createElement('textarea');
      ta.value = code;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand('copy');
      document.body.removeChild(ta);
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  }

  return (
    <div className="codeblock">
      <header>
        <span className="dots"><i /><i /><i /></span>
        <span className="lang">{lang}</span>
        {title && <span className="grow">{title}</span>}
        {!title && <span className="grow" />}
        {snippetId && SNIPPETS[snippetId] && (
          <button className="btn" onClick={() => nav('#/playground/' + snippetId)} title="Edit and run this program">
            Playground
          </button>
        )}
        <button className="btn" onClick={copy}>
          {copied ? 'Copied' : 'Copy'}
        </button>
        {runnable && (
          <button className="btn primary" onClick={run} disabled={running}>
            {running ? 'Running…' : 'Run'}
          </button>
        )}
      </header>
      <pre dangerouslySetInnerHTML={{ __html: lang === 'sn' ? highlight(code) : escapeHtml(code) }} />
      {note && (
        <div className="note-row">{note}</div>
      )}
      {output && (
        <details className="expected">
          <summary>Expected output</summary>
          <pre>{output}</pre>
        </details>
      )}
      {result && (
        <div className="runout">
          <span className={result.ok ? 'ok' : 'bad'}>{result.ok ? 'exit 0' : 'failed'}</span>
          {result.out && <pre>{result.out}</pre>}
          {result.ms != null && <div className="meta">exit_code={result.code} · {result.ms} ms · compiled and ran by snc</div>}
        </div>
      )}
    </div>
  );
}

function escapeHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}
