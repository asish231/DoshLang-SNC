import React from 'react';
import CodeBlock from '../components/CodeBlock.jsx';
import { api } from '../sn.jsx';
import { SNIPPETS, OUTPUTS } from '../data/snippets.js';
import { CURATED, DSA } from '../data/ref.js';

export default function Examples() {
  const [names, setNames] = React.useState(null);
  const [namesErr, setNamesErr] = React.useState(null);
  const [open, setOpen] = React.useState(null); // {kind:'repo', name, code} | {kind:'dsa', ...}
  const [loading, setLoading] = React.useState(false);
  const [q, setQ] = React.useState('');

  React.useEffect(() => {
    api.examples().then(setNames).catch((e) => setNamesErr(String((e && e.message) || e)));
  }, []);

  async function openRepo(name) {
    setLoading(true);
    try {
      const d = await api.example(name);
      setOpen({ kind: 'repo', name: d.name, code: d.code });
    } catch (e) {
      setOpen({ kind: 'repo', name, code: '// could not load: ' + (e.message || e) });
    } finally {
      setLoading(false);
    }
  }

  async function openDsa(entry) {
    setLoading(true);
    try {
      const d = await api.dsa(entry.id);
      setOpen({ kind: 'dsa', entry, code: d.code });
    } catch {
      setOpen({ kind: 'dsa', entry, code: SNIPPETS[entry.snippet] });
    } finally {
      setLoading(false);
    }
  }

  const needle = q.trim().toLowerCase();
  const curated = CURATED.filter(
    (c) => !needle || c.title.toLowerCase().includes(needle) || c.desc.toLowerCase().includes(needle) || c.name.includes(needle)
  );

  return (
    <div>
      <h1>Examples</h1>
      <p className="lede">
        Real programs from this repository. Curated entries below all compile and run clean — open one to read it,
        run it, or send it to the playground. The full directory listing at the bottom is served live by the server.
      </p>

      <h2>Algorithms in SNlang</h2>
      <p>Served by this site's own backend (<code>GET /api/dsa/:id</code>) — itself a runnable SNlang program.</p>
      <div className="ex-list">
        {DSA.map((d) => (
          <div key={d.id} className={'ex-item' + (open && open.kind === 'dsa' && open.entry.id === d.id ? ' active' : '')} onClick={() => openDsa(d)}>
            <b>{d.title}</b>
            <p>{d.desc}</p>
            <div className="tags">
              <span className="tag">{d.cat}</span>
              <span className="tag">{d.diff}</span>
              <span className="tag">{d.time}</span>
            </div>
          </div>
        ))}
      </div>

      <h2>Curated programs</h2>
      <input className="search" placeholder="Filter… e.g. channel, json, map" value={q} onChange={(e) => setQ(e.target.value)} />
      <div className="two-col">
        <div className="ex-list">
          {curated.map((c) => (
            <div key={c.name} className={'ex-item' + (open && open.kind === 'repo' && open.name === c.name ? ' active' : '')} onClick={() => openRepo(c.name)}>
              <b>{c.title}</b>
              <p>{c.desc}</p>
              <div className="tags">{c.tags.map((t) => <span key={t} className="tag">{t}</span>)}</div>
            </div>
          ))}
          {curated.length === 0 && <p>No curated examples match “{q}”.</p>}
        </div>
        <div>
          {loading && <p style={{ color: 'var(--muted)' }}>Loading…</p>}
          {!open && !loading && <p style={{ color: 'var(--muted)' }}>Select a program to read and run it.</p>}
          {open && open.kind === 'repo' && (
            <CodeBlock
              key={open.name}
              code={open.code}
              title={open.name}
              runnable={!open.code.startsWith('// could not')}
              snippetId={undefined}
            />
          )}
          {open && open.kind === 'dsa' && (
            <CodeBlock
              key={open.entry.id}
              code={open.code}
              title={open.entry.title + ' · ' + open.entry.time}
              runnable
              output={open.entry.unordered ? null : OUTPUTS[open.entry.snippet]}
              snippetId={open.entry.snippet}
              note={open.entry.unordered ? 'Worker threads race, so the three lines can arrive in any order.' : null}
            />
          )}
        </div>
      </div>

      <h2>Everything in examples/</h2>
      {namesErr && (
        <div className="callout warn">
          <b>Live listing unavailable</b>
          {namesErr} — the curated programs above still work because their code ships with the page.
        </div>
      )}
      {names && (
        <p style={{ color: 'var(--muted)', fontSize: 14 }}>
          {names.length} SNlang files on the server. Anything listed here can be opened read-only; curated ones are verified to run.
        </p>
      )}
      {names && (
        <div className="ex-list">
          {names.map((n) => (
            <div
              key={n.name}
              className={'ex-item' + (open && open.kind === 'repo' && open.name === n.name ? ' active' : '')}
              onClick={() => openRepo(n.name)}
            >
              <b style={{ fontFamily: 'var(--font-mono)', fontSize: 14 }}>{n.name}</b>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
