import React from 'react';
import CodeBlock from '../components/CodeBlock.jsx';
import { CLI } from '../data/ref.js';

export default function Tooling() {
  return (
    <div>
      <h1>Tooling reference</h1>
      <p className="lede">
        Everything <code>snc</code> does besides compiling: formatting, testing, packages, translation, editor support, and debugging.
      </p>

      {CLI.map((c) => (
        <div key={c.cmd}>
          <h2 style={{ fontFamily: 'var(--font-mono)', fontSize: 19 }}><code>{c.cmd}</code></h2>
          <p>{c.desc}</p>
          <CodeBlock lang="sh" code={c.code} />
        </div>
      ))}

      <h2>Package workflow end to end</h2>
      <p>
        A package is a folder with <code>sn.toml</code>. Depend on a local folder, a standard package, or a registry tarball;
        commit <code>sn.lock.toml</code> so builds reproduce. Hosting a registry is one static file server away — see{' '}
        <code>docs/PACKAGES.md</code>.
      </p>
      <CodeBlock
        lang="sh"
        code={'snc pkg init --name myapp\nsnc pkg add mylib --path packages/mylib\nsnc pkg add foo --registry https://example.com/snlang --version 1.0.0\nsnc pkg list\nsnc pkg build -o app\nsnc pkg publish   # dist/<name>-<ver>.tar.gz'}
      />

      <h2>Editor support</h2>
      <p>
        <code>snc lsp</code> speaks Language Server Protocol over stdio: diagnostics with line/column ranges, completion
        (locals, functions, blueprints, and methods after <code>d.</code>), hover signatures, go-to-definition, find
        references, rename, and document symbols. Pair it with the <code>vscode-snlang/</code> grammar in this repo for
        highlighting.
      </p>

      <h2>Troubleshooting</h2>
      <div className="callout warn">
        <b>cargo: command not found</b>
        Rust is installed but not on PATH. Run <code>export PATH="$HOME/.cargo/bin:$PATH"</code> (the Makefile does this for its own recipes, but your shell needs it too).
      </div>
      <div className="callout warn">
        <b>clang errors about missing headers</b>
        Install a full toolchain: Xcode command-line tools on macOS, or <code>build-essential</code> plus <code>clang</code> on Linux.
      </div>
      <div className="callout warn">
        <b>Playground says it cannot reach the server</b>
        The site UI is static until the SNlang backend runs: <code>./snc examples/website/server.sn -o website_server &amp;&amp; ./website_server</code>, then reload port 8090.
      </div>

      <h2>This site runs on SNlang</h2>
      <p>
        The backend serving these pages is <code>examples/website/server.sn</code>: routing, middleware, SQLite persistence,
        rate limiting, and the sandbox that powers every Run button — all SNlang, all native. Its JSON API:
      </p>
      <CodeBlock
        lang="sh"
        code={'GET  /api/healthz /api/readyz /api/status /api/features /api/telemetry\nGET  /api/docs/examples /api/docs/examples/:name\nGET  /api/dsa /api/dsa/:id\nPOST /api/sandbox/run            # {"code": "..."} -> {output, exit_code, duration_ms}\nPOST /api/projects ...          # save snippets (login required)\nPOST /api/ai/chat               # offline rule-based SNlang tutor\nPOST /api/ai/explain            # {"code": "..."} -> {reply} (Gemini, needs GEMINI_API_KEY)\nPOST /api/ai/try                # {"provider","key","input","model"} -> {reply} (your key)}'}
      />
    </div>
  );
}
