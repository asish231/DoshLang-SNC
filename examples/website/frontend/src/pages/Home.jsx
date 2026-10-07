import React from 'react';
import CodeBlock from '../components/CodeBlock.jsx';
import { SNIPPETS, OUTPUTS } from '../data/snippets.js';
import { CHAPTERS } from '../data/guide.js';

export default function Home() {
  return (
    <div>
      <div className="home-overline">SNlang documentation</div>
      <h1 className="home-title">A small typed language that compiles to native code</h1>
      <p className="home-lede">
        SNlang has plain syntax (<code>fn</code>, braces, <code>and</code> / <code>or</code> / <code>not</code>) and a
        compiler written in Rust that emits LLVM IR and links it with clang. This site takes you from installation to
        writing concurrent programs — every example runs in your browser against the real compiler.
      </p>
      <CodeBlock
        lang="sh"
        title="install & build"
        code={'export PATH="$HOME/.cargo/bin:$PATH"\nmake                      # builds ./snc\n./snc examples/hello_world.sn -o hello && ./hello'}
      />
      <div className="home-cta">
        <a className="btn primary big" href="#/start">Get started</a>
        <a className="btn big" href="#/guide/basics">Language guide</a>
        <a className="btn big" href="#/playground">Playground</a>
      </div>

      <h2>Start here</h2>
      <ol className="steps">
        <li data-n="1">
          <b><a href="#/start">Install and build the compiler</a></b>
          <p>Rust, clang, and one <code>make</code> command. About five minutes.</p>
        </li>
        <li data-n="2">
          <b><a href="#/guide/basics">Follow the language guide</a></b>
          <p>Fourteen chapters, zero to channels. Each program is runnable on this page.</p>
        </li>
        <li data-n="3">
          <b><a href="#/examples">Read real programs</a></b>
          <p>Curated repository examples and six algorithms, served live from the repo.</p>
        </li>
        <li data-n="4">
          <b><a href="#/tooling">Learn the tooling</a></b>
          <p><code>fmt</code>, <code>test</code>, <code>pkg</code>, <code>translate</code>, <code>lsp</code>, <code>debug</code>.</p>
        </li>
      </ol>

      <h2>Try it now</h2>
      <p>The classic first program. Press <b>Run</b> to compile and execute it on this site's SNlang server.</p>
      <CodeBlock code={SNIPPETS.hello} runnable snippetId="hello" output={OUTPUTS.hello} />

      <h2>Language guide</h2>
      <table className="idx">
        <tbody>
          {CHAPTERS.map((c) => (
            <tr key={c.id}>
              <td className="n">{c.num}</td>
              <td className="t"><a href={'#/guide/' + c.id}>{c.title}</a></td>
              <td className="d">{c.tagline}</td>
            </tr>
          ))}
        </tbody>
      </table>

      <h2>Reference</h2>
      <div className="reflinks">
        <a href="#/stdlib">Standard library</a>
        <a href="#/examples">Examples</a>
        <a href="#/tooling">Tooling &amp; API</a>
        <a href="#/playground">Playground</a>
      </div>
    </div>
  );
}
