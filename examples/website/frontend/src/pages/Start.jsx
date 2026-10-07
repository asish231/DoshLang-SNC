import React from 'react';
import CodeBlock from '../components/CodeBlock.jsx';
import { nav } from '../sn.jsx';
import { SNIPPETS, OUTPUTS } from '../data/snippets.js';

export default function Start() {
  return (
    <div>
      <h1>Get started</h1>
      <p className="lede">From an empty machine to running your first SNlang program in about five minutes.</p>

      <h2>1 · Prerequisites</h2>
      <p>You need Rust (for building the compiler) and clang (for linking native binaries):</p>
      <CodeBlock
        lang="sh"
        code={'# Rust toolchain (provides cargo)\ncurl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh\n\n# clang: macOS ships it with Xcode tools; Debian/Ubuntu:\n# sudo apt install clang\n\ncargo --version\nclang --version'}
      />

      <h2>2 · Build the compiler</h2>
      <p>Clone the repository and run make. This produces <code>./snc</code> in the project root:</p>
      <CodeBlock
        lang="sh"
        code={'export PATH="$HOME/.cargo/bin:$PATH"\nmake          # builds ./snc\nmake test   # compiler tests + hello-world smoke run'}
      />
      <div className="callout">
        <b>Windows</b>
        Run <code>./build.ps1</code> instead of <code>make</code>; it produces <code>snc.exe</code>.
      </div>

      <h2>3 · Compile and run hello</h2>
      <p>
        <code>snc</code> takes a <code>.sn</code> file and emits a native executable with <code>-o</code>. Run the bundled
        hello world first so you know the toolchain works:
      </p>
      <CodeBlock lang="sh" code={'./snc examples/hello_world.sn -o hello\n./hello\n# Hello, World!'} />
      <p>This is the program you just ran:</p>
      <CodeBlock code={SNIPPETS.hello} runnable snippetId="hello" output={OUTPUTS.hello} />

      <h2>4 · Write your own</h2>
      <p>
        Create <code>main.sn</code> anywhere, paste the printing tour below into it, and compile it the same way. The
        module search starts in your file's directory, so <code>use std.math</code> and friends resolve without any setup:
      </p>
      <CodeBlock code={SNIPPETS.printing} runnable snippetId="printing" output={OUTPUTS.printing} />
      <CodeBlock lang="sh" code={'./snc main.sn -o myapp\n./myapp'} />

      <h2>5 · Keep learning</h2>
      <div className="reflinks">
        <a href="#/guide/basics">Language guide →</a>
        <a href="#/playground">Playground →</a>
        <a href="#/tooling">Tooling reference →</a>
      </div>

      <div className="callout good">
        <b>Editor support</b>
        The repo ships a VSCode grammar (<code>vscode-snlang/</code>) with keyword highlighting, and <code>snc lsp</code> speaks
        Language Server Protocol over stdio: diagnostics, completion, hover, go-to-definition, references, rename, symbols.
      </div>

      <div className="pager">
        <a href="#/" onClick={(e) => { e.preventDefault(); nav('#/'); }}>
          <span>← Previous</span><strong>Home</strong>
        </a>
        <a href="#/guide/basics" onClick={(e) => { e.preventDefault(); nav('#/guide/basics'); }}>
          <span>Next →</span><strong>01 · Your first program</strong>
        </a>
      </div>
    </div>
  );
}
