import React from 'react';
import { api, cleanOutput } from '../sn.jsx';
import { SNIPPETS, PRESETS } from '../data/snippets.js';

const PROVIDERS = {
  google: { label: 'Google', model: 'gemini-2.5-flash', key_hint: 'AIza… (AI Studio)' },
  openai: { label: 'OpenAI', model: 'gpt-5', key_hint: 'sk-…' },
  anthropic: { label: 'Anthropic', model: 'claude-sonnet-4-6', key_hint: 'sk-ant-…' },
};

function escSn(s) {
  return String(s).replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/, '');
}

function implCode(provider, key, model, input) {
  const k = escSn(key);
  const m = escSn(model);
  const inp = escSn(input);
  if (provider === 'google') {
    return 'use std.ai\n\nfn main() {\n    GoogleClient client = google_client_with("' + k + '", "' + m + '")\n    str text = client.interactions_create("' + inp + '")\n    print(text)\n}';
  }
  if (provider === 'openai') {
    return 'use std.ai\n\nfn main() {\n    OpenAIClient client = openai_client_with("' + k + '", "' + m + '")\n    str text = client.responses_create("' + inp + '")\n    print(text)\n}';
  }
  return 'use std.ai\n\nfn main() {\n    AnthropicClient client = anthropic_client_with("' + k + '", "' + m + '")\n    str text = client.messages_create("' + inp + '")\n    print(text)\n}';
}

export default function Ai({ presetId }) {
  const [tab, setTab] = React.useState('explain');
  return (
    <div>
      <h1>AI Playground</h1>
      <div className="tabs">
        <button className={'btn' + (tab === 'explain' ? ' on' : '')} onClick={() => setTab('explain')}>Explain code</button>
        <button className={'btn' + (tab === 'sdk' ? ' on' : '')} onClick={() => setTab('sdk')}>SDK demo</button>
      </div>
      {tab === 'explain' ? <Explain presetId={presetId} /> : <SdkDemo />}
    </div>
  );
}

function Explain({ presetId }) {
  const startId = presetId && SNIPPETS[presetId] ? presetId : 'funcs';
  const [code, setCode] = React.useState(SNIPPETS[startId]);
  const [preset, setPreset] = React.useState(startId);
  const [asking, setAsking] = React.useState(false);
  const [reply, setReply] = React.useState(null);

  function pick(id) {
    setPreset(id);
    setCode(SNIPPETS[id] || '');
    setReply(null);
  }

  async function ask() {
    setAsking(true);
    setReply(null);
    try {
      const r = await api.explain(code);
      setReply({ ok: true, text: r.reply });
    } catch (e) {
      const msg = String((e && e.message) || e);
      setReply({
        ok: false,
        text: msg.includes('Failed to fetch')
          ? 'Cannot reach the SNlang server. Start it (see Get Started) and reload.'
          : msg,
      });
    } finally {
      setAsking(false);
    }
  }

  return (
    <div>
      <p className="lede">
        Paste SNlang on the left, get a beginner-friendly explanation on the right — answered by Gemini, called from
        this site's own SNlang backend.
      </p>
      <div className="callout">
        <b>How it works</b>
        Your code is sent to <code>POST /api/ai/explain</code>, which forwards it to Gemini with the key from the
        server's <code>.env</code> (<code>GEMINI_API_KEY</code>, never committed, never logged). Limited to 20
        requests per minute.
      </div>
      <div className="playbar">
        <select value={preset} onChange={(e) => pick(e.target.value)}>
          {PRESETS.map((p) => (
            <option key={p.id} value={p.id}>
              {p.title}
            </option>
          ))}
          {!PRESETS.some((p) => p.id === preset) && <option value={preset}>Guide snippet: {preset}</option>}
        </select>
        <button className="btn primary" onClick={ask} disabled={asking}>
          {asking ? 'Asking Gemini…' : 'Explain this code'}
        </button>
        <span style={{ color: 'var(--faint)', fontSize: 13 }}>model: gemini-2.5-flash (override with GEMINI_MODEL)</span>
      </div>
      <div className="play">
        <textarea value={code} onChange={(e) => setCode(e.target.value)} spellCheck={false} />
        <div className="pane">
          <header>
            <span style={{ fontWeight: 700, color: reply ? (reply.ok ? 'var(--accent)' : 'var(--red)') : 'var(--muted)' }}>
              {asking ? 'thinking…' : reply ? (reply.ok ? 'explanation' : 'failed') : 'explanation'}
            </span>
          </header>
          <div className="body">{reply ? reply.text || '(empty reply)' : 'Press “Explain this code” to ask Gemini.'}</div>
        </div>
      </div>
    </div>
  );
}

function SdkDemo() {
  const [provider, setProvider] = React.useState('google');
  const [key, setKey] = React.useState(() => localStorage.getItem('sn_ai_key_google') || '');
  const [model, setModel] = React.useState(PROVIDERS.google.model);
  const [input, setInput] = React.useState('Explain how AI works in a few words');
  const [code, setCode] = React.useState(() => implCode('google', localStorage.getItem('sn_ai_key_google') || 'YOUR_API_KEY', PROVIDERS.google.model, 'Explain how AI works in a few words'));
  const [dirty, setDirty] = React.useState(false);
  const [running, setRunning] = React.useState(false);
  const [out, setOut] = React.useState(null);

  function regen(p, k, m, i) {
    if (!dirty) setCode(implCode(p, k || 'YOUR_API_KEY', m, i));
  }

  function pickProvider(p) {
    setProvider(p);
    const k = localStorage.getItem('sn_ai_key_' + p) || '';
    const m = PROVIDERS[p].model;
    setKey(k);
    setModel(m);
    setOut(null);
    regen(p, k, m, input);
  }

  function saveKey(v) {
    setKey(v);
    if (v) localStorage.setItem('sn_ai_key_' + provider, v);
    else localStorage.removeItem('sn_ai_key_' + provider);
    regen(provider, v, model, input);
  }

  function changeModel(v) {
    setModel(v);
    regen(provider, key, v, input);
  }

  function changeInput(v) {
    setInput(v);
    regen(provider, key, model, v);
  }

  function resetCode() {
    setCode(implCode(provider, key || 'YOUR_API_KEY', model, input));
    setDirty(false);
  }

  async function run() {
    setRunning(true);
    setOut(null);
    try {
      const r = await api.run(code);
      const text = cleanOutput(r.output);
      setOut({ ok: r.exit_code === 0 && text.length > 0, text: text || '(empty reply — check the key and model)', ms: r.duration_ms, code: r.exit_code });
    } catch (e) {
      const msg = String((e && e.message) || e);
      setOut({
        ok: false,
        text: msg.includes('Failed to fetch')
          ? 'Cannot reach the SNlang server. Start it (see Get Started) and reload.'
          : msg,
      });
    } finally {
      setRunning(false);
    }
  }

  return (
    <div>
      <p className="lede">
        Use <b>your own API key</b> with the <code>std.ai</code> SDK. The fields generate the starter program on the
        left — then <b>edit it freely</b>: try other models, multi-turn chats, loops. Run compiles and executes
        exactly what's in the editor, output on the right.
      </p>
      <div className="callout">
        <b>Your key stays yours</b>
        It is kept only in this browser (<code>localStorage</code>), sent to this server solely to make the provider
        call, and never stored or logged server-side. Clear it any time with the × button.
      </div>
      <div className="field">
        <label>Provider</label>
        <div className="pills">
          {Object.keys(PROVIDERS).map((p) => (
            <button key={p} className={'btn' + (provider === p ? ' on' : '')} onClick={() => pickProvider(p)}>
              {PROVIDERS[p].label}
            </button>
          ))}
        </div>
      </div>
      <div className="field">
        <label>API key</label>
        <input
          className="search"
          type="password"
          placeholder={PROVIDERS[provider].key_hint}
          value={key}
          onChange={(e) => saveKey(e.target.value)}
        />
        {key && <button className="btn" onClick={() => saveKey('')} title="Forget this key">×</button>}
      </div>
      <div className="field">
        <label>Model</label>
        <input className="search" value={model} onChange={(e) => changeModel(e.target.value)} spellCheck={false} />
      </div>
      <div className="field">
        <label>Prompt</label>
        <input className="search" value={input} onChange={(e) => changeInput(e.target.value)} spellCheck={false} />
      </div>
      <div className="playbar">
        <button className="btn primary" onClick={run} disabled={running || !key}>
          {running ? 'Compiling & running…' : 'Run this code'}
        </button>
        {dirty && <button className="btn" onClick={resetCode}>Reset to generated code</button>}
        <span style={{ color: 'var(--faint)', fontSize: 13 }}>
          {dirty ? 'customized — runs exactly as written' : 'edit the code freely, Run executes it via snc'} · spends your quota
        </span>
      </div>
      <div className="play">
        <textarea value={code} onChange={(e) => { setCode(e.target.value); setDirty(true); }} spellCheck={false} />
        <div className="pane">
          <header>
            <span style={{ fontWeight: 700, color: out ? (out.ok ? 'var(--accent)' : 'var(--red)') : 'var(--muted)' }}>
              {running ? 'running…' : out ? (out.ok ? 'exit 0' : 'failed') : 'output'}
            </span>
            {out && out.ms != null && <span style={{ color: 'var(--faint)', fontWeight: 400 }}>exit_code={out.code} · {out.ms} ms</span>}
          </header>
          <div className="body">{out ? out.text : 'Press “Run this code” — it compiles and executes the editor content.'}</div>
        </div>
      </div>
    </div>
  );
}
