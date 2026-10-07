import React from 'react';
import CodeBlock from '../components/CodeBlock.jsx';
import Rich from '../components/Rich.jsx';
import { nav } from '../sn.jsx';
import { SNIPPETS, OUTPUTS } from '../data/snippets.js';
import { CHAPTERS } from '../data/guide.js';

export default function Guide({ chapterId }) {
  const idx = CHAPTERS.findIndex((c) => c.id === chapterId);
  if (idx === -1) {
    return (
      <div>
        <h1>Language guide</h1>
        <p className="lede">
          Fourteen chapters, zero to productive. Each one teaches a single idea with short programs you can run right
          here — every snippet is compiled by the real <code>snc</code> before it ships on this page.
        </p>
        <div className="ex-list">
          {CHAPTERS.map((c) => (
            <div key={c.id} className="ex-item" onClick={() => nav('#/guide/' + c.id)}>
              <b><span className="chapter-num">{c.num}</span> · {c.title}</b>
              <p>{c.tagline}</p>
              <div className="tags">
                <span className="tag">{c.sections.length} sections</span>
                <span className="tag">{c.sections.filter((s) => s.snippet).length} runnable</span>
              </div>
            </div>
          ))}
        </div>
      </div>
    );
  }

  const ch = CHAPTERS[idx];
  const prev = CHAPTERS[idx - 1];
  const next = CHAPTERS[idx + 1];

  return (
    <div>
      <div className="chapter-meta">
        <span className="chapter-num">Chapter {ch.num} / 14</span>
        <a href="#/guide" onClick={(e) => { e.preventDefault(); nav('#/guide'); }} style={{ fontSize: 13 }}>All chapters</a>
      </div>
      <h1>{ch.title}</h1>
      <p className="lede">{ch.tagline}</p>

      {ch.sections.map((s, i) => (
        <section key={i}>
          <h2>{s.h}</h2>
          <Rich text={s.body} />
          {s.snippet && SNIPPETS[s.snippet] && (
            <CodeBlock
              code={SNIPPETS[s.snippet]}
              runnable={s.runnable !== false}
              output={s.output ? OUTPUTS[s.output] : null}
              snippetId={s.snippet}
              note={s.runnable === false ? 'Static sample — run it locally with ./snc, since executing it has side effects.' : null}
            />
          )}
        </section>
      ))}

      <div className="callout good">
        <b>Try it yourself</b>
        Every program above has a <b>Playground</b> button. Open it, break the code on purpose, and read the compiler
        error — learning to read errors is half of learning the language.
      </div>

      <div className="pager">
        {prev ? (
          <a href={'#/guide/' + prev.id} onClick={(e) => { e.preventDefault(); nav('#/guide/' + prev.id); }}>
            <span>← Previous</span><strong>{prev.num} · {prev.title}</strong>
          </a>
        ) : <span />}
        {next ? (
          <a href={'#/guide/' + next.id} onClick={(e) => { e.preventDefault(); nav('#/guide/' + next.id); }}>
            <span>Next →</span><strong>{next.num} · {next.title}</strong>
          </a>
        ) : (
          <a href="#/stdlib" onClick={(e) => { e.preventDefault(); nav('#/stdlib'); }}>
            <span>Next →</span><strong>Standard library reference</strong>
          </a>
        )}
      </div>
    </div>
  );
}
