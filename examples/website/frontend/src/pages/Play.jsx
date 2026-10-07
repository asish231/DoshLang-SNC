import React from 'react';
import Playground from '../components/Playground.jsx';

export default function Play({ presetId }) {
  return (
    <div>
      <h1>Playground</h1>
      <p className="lede">
        Write SNlang and press Run. Your code is compiled to a native binary by <code>snc</code> on this site's server
        and the program output comes back here — the same pipeline <code>./snc file.sn -o app</code> runs locally.
      </p>
      <div className="callout">
        <b>Fair warning</b>
        This executes real code on the demo server: keep snippets small (32 KB limit), stick to the standard library,
        and avoid network calls or infinite loops. Anything you run here you could run at home with zero setup beyond the compiler.
      </div>
      <Playground key={presetId || 'default'} presetId={presetId} />
    </div>
  );
}
