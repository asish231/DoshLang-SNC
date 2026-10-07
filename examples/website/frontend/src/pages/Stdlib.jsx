import React from 'react';
import { STDLIB } from '../data/ref.js';

export default function Stdlib() {
  const [q, setQ] = React.useState('');
  const needle = q.trim().toLowerCase();
  const pkgs = STDLIB.map((p) => ({
    ...p,
    fns: p.fns.filter(
      ([sig, desc]) => !needle || sig.toLowerCase().includes(needle) || desc.toLowerCase().includes(needle) || p.pkg.includes(needle)
    ),
  })).filter((p) => p.fns.length > 0);

  return (
    <div>
      <h1>Standard library</h1>
      <p className="lede">
        Fifteen packages ship with the compiler under <code>stdlib/std/</code>. Import with <code>use std.math</code> —
        lookup starts in your file's directory, then <code>packages/</code>, then <code>stdlib/</code>.
      </p>
      <input className="search" placeholder="Filter functions… e.g. sha256, channel, parse" value={q} onChange={(e) => setQ(e.target.value)} />
      {pkgs.length === 0 && <p>No functions match “{q}”.</p>}
      {pkgs.map((p) => (
        <div key={p.pkg} className="pkg">
          <h3>{p.pkg}</h3>
          <p style={{ color: 'var(--muted)', fontSize: 14, marginTop: -6 }}>{p.desc}</p>
          <table className="ref">
            <thead>
              <tr><th>Signature</th><th>What it does</th></tr>
            </thead>
            <tbody>
              {p.fns.map(([sig, desc]) => (
                <tr key={sig}><td className="sig">{sig}</td><td>{desc}</td></tr>
              ))}
            </tbody>
          </table>
        </div>
      ))}
      <div className="callout">
        <b>Globals need no import</b>
        <code>print</code>, <code>printn</code>, <code>input</code>, <code>cast</code>, <code>select</code>, <code>time_sleep_ms</code>,
        file helpers (<code>file_read</code>, <code>file_write</code>, <code>file_append</code>), async futures
        (<code>sleep_async</code>, <code>get_async</code>), GC controls (<code>gc_collect</code>, <code>gc_count</code>),
        and raw-memory guards (<code>ptr_alloc</code>, <code>ptr_free</code>, <code>check_live</code>) are always available.
      </div>
    </div>
  );
}
