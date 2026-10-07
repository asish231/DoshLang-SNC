// Shared helpers: hash routing, backend API, SNlang highlighting, mini-markdown.
import React from 'react';

export function parseHash() {
  const h = (window.location.hash || '#/').replace(/^#/, '');
  const parts = h.split('/').filter(Boolean);
  return { page: parts[0] || 'home', param: decodeURIComponent(parts[1] || '') };
}

export function nav(to) {
  window.location.hash = to;
}

export function useRoute() {
  const [route, setRoute] = React.useState(parseHash());
  React.useEffect(() => {
    const onChange = () => {
      setRoute(parseHash());
      window.scrollTo(0, 0);
    };
    window.addEventListener('hashchange', onChange);
    return () => window.removeEventListener('hashchange', onChange);
  }, []);
  return route;
}

async function req(path, opts) {
  const res = await fetch(path, opts);
  const text = await res.text();
  let data = null;
  try {
    data = JSON.parse(text);
  } catch {
    data = { raw: text };
  }
  if (!res.ok) {
    const msg = (data && data.error) || ('HTTP ' + res.status);
    throw new Error(msg);
  }
  return data;
}

export const api = {
  run(code) {
    return req('/api/sandbox/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code }),
    });
  },
  examples() {
    return req('/api/docs/examples');
  },
  example(name) {
    return req('/api/docs/examples/' + encodeURIComponent(name));
  },
  dsaList() {
    return req('/api/dsa');
  },
  dsa(id) {
    return req('/api/dsa/' + encodeURIComponent(id));
  },
  explain(code) {
    return req('/api/ai/explain', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ code }),
    });
  },
  tryAi({ provider, key, input, model }) {
    return req('/api/ai/try', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ provider, key, input, model }),
    });
  },
  health() {
    return req('/api/healthz');
  },
};

// Drop the compiler's own chatter so learners only see program output.
export function cleanOutput(out) {
  if (!out) return '';
  return out
    .split('\n')
    .filter((l) => !/^wrote \S+$/.test(l.trim()))
    .join('\n')
    .trim();
}

// ---- tiny SNlang syntax highlighter ----
const KEYWORDS = new Set(
  'fn blueprint enum contract use if else match default for in while return spawn goroutine new self super from static abstract closed guarded ref mut address value try panic defer stop skip extern asm let true false none and or not as'.split(' ')
);
const TYPES = new Set(
  'int str bool list map ptr json error chan lock any record byte dec float fn'.split(' ')
);
const BUILTINS = new Set(
  'print println cast len select spawn_chan make go'.split(' ')
);

function esc(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

export function highlight(code) {
  const re = /(\/\/[^\n]*)|("(?:[^"\\\n]|\\.)*")|\b(\d+(?:\.\d+)?)\b|\b([A-Za-z_][A-Za-z0-9_?!]*)\b|(\s+|.)/g;
  let html = '';
  let m;
  while ((m = re.exec(code)) !== null) {
    const [tok, comment, str, num, word, other] = m;
    if (comment) html += '<span class="tok-c">' + esc(tok) + '</span>';
    else if (str) html += '<span class="tok-s">' + esc(tok) + '</span>';
    else if (num) html += '<span class="tok-n">' + esc(tok) + '</span>';
    else if (word) {
      if (KEYWORDS.has(word)) html += '<span class="tok-k">' + esc(tok) + '</span>';
      else if (TYPES.has(word)) html += '<span class="tok-t">' + esc(tok) + '</span>';
      else if (BUILTINS.has(word)) html += '<span class="tok-b">' + esc(tok) + '</span>';
      else html += esc(tok);
    } else html += esc(other || tok);
  }
  return html;
}

// ---- mini markdown: paragraphs, "- " bullets, `inline code` ----
// (Rich lives in components/Rich.jsx so fast-refresh stays happy.)

export function inlineCode(s) {
  return esc(s).replace(/`([^`]+)`/g, '<code>$1</code>');
}
