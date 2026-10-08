import { useEffect } from 'react';
import { nav, useRoute } from './sn.jsx';
import { CHAPTERS } from './data/guide.js';
import Home from './pages/Home.jsx';
import Start from './pages/Start.jsx';
import Guide from './pages/Guide.jsx';
import Play from './pages/Play.jsx';
import Ai from './pages/Ai.jsx';
import Stdlib from './pages/Stdlib.jsx';
import Examples from './pages/Examples.jsx';
import Tooling from './pages/Tooling.jsx';

const TOP = [
  ['#/', 'home', 'Home'],
  ['#/start', 'start', 'Get Started'],
  ['#/guide', 'guide', 'Guide'],
  ['#/playground', 'playground', 'Playground'],
  ['#/ai', 'ai', 'AI Playground'],
  ['#/stdlib', 'stdlib', 'Stdlib'],
  ['#/examples', 'examples', 'Examples'],
  ['#/tooling', 'tooling', 'Tooling'],
];

export default function App() {
  const route = useRoute();

  useEffect(() => {
    const titles = {
      home: 'SNlang — High-Performance Native Systems Programming Language',
      start: 'Install & Get Started — SNlang Documentation',
      guide: route.param ? `Language Guide: ${route.param} — SNlang` : 'Complete Language Guide — SNlang',
      playground: 'Interactive Live Compiler Playground — SNlang',
      ai: 'AI Playground & Intelligent Code Assistant — SNlang',
      stdlib: 'Standard Library Reference (std.*) — SNlang',
      examples: 'Curated Code Examples & Algorithms — SNlang',
      tooling: 'Compiler Tooling, Package Manager & CLI — SNlang',
    };
    const title = titles[route.page] || 'SNlang — High-Performance Native Systems Programming Language';
    document.title = title;
    if (typeof window !== 'undefined' && typeof window.gtag === 'function') {
      window.gtag('event', 'page_view', {
        page_title: title,
        page_location: window.location.href,
        page_path: window.location.hash || '#/',
      });
    }
  }, [route]);

  let body;
  if (route.page === 'start') body = <Start />;
  else if (route.page === 'guide') body = <Guide chapterId={route.param} />;
  else if (route.page === 'playground') body = <Play presetId={route.param} />;
  else if (route.page === 'ai') body = <Ai presetId={route.param} />;
  else if (route.page === 'stdlib') body = <Stdlib />;
  else if (route.page === 'examples') body = <Examples />;
  else if (route.page === 'tooling') body = <Tooling />;
  else body = <Home />;

  const activeTop = route.page === 'home' || route.page === '' ? 'home' : route.page;

  return (
    <div>
      <div className="topbar">
        <span className="brand" onClick={() => nav('#/')}>
          <img src="/mascot.png" alt="SNlang" className="brand-logo" />
          SNlang
          <small>starter &amp; docs</small>
        </span>
        <nav>
          {TOP.map(([to, id, label]) => (
            <a key={id} href={to} className={activeTop === id ? 'active' : ''} onClick={(e) => { e.preventDefault(); nav(to); }}>
              {label}
            </a>
          ))}
        </nav>
        <span className="spacer" />
        <div className="topbar-actions">
          <a
            href="https://github.com/asish231/DoshLang-SNC/releases"
            target="_blank"
            rel="noopener noreferrer"
            className="btn-download-nav"
            title="Download prebuilt binaries"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            Download
          </a>
          <a
            href="https://github.com/asish231/DoshLang-SNC"
            target="_blank"
            rel="noopener noreferrer"
            className="btn-github-nav"
            title="GitHub Repository"
          >
            <svg width="15" height="15" viewBox="0 0 19 19" fill="currentColor">
              <use href="/icons.svg#github-icon" />
            </svg>
            GitHub
          </a>
        </div>
      </div>

      <div className="shell">
        <aside className="sidebar">
          <h5>Start here</h5>
          <SideLink to="#/" active={activeTop === 'home'} label="Welcome" />
          <SideLink to="#/start" active={route.page === 'start'} label="Install & first run" />
          <h5>Language guide</h5>
          {CHAPTERS.map((c) => (
            <SideLink key={c.id} to={'#/guide/' + c.id} active={route.page === 'guide' && route.param === c.id} label={c.title} num={c.num} />
          ))}
          <h5>Reference</h5>
          <SideLink to="#/playground" active={route.page === 'playground'} label="Playground" />
          <SideLink to="#/ai" active={route.page === 'ai'} label="AI Playground" />
          <SideLink to="#/stdlib" active={route.page === 'stdlib'} label="Standard library" />
          <SideLink to="#/examples" active={route.page === 'examples'} label="Examples" />
          <SideLink to="#/tooling" active={route.page === 'tooling'} label="Tooling & API" />
          <h5>Project</h5>
          <a href="https://github.com/asish231/DoshLang-SNC/releases" target="_blank" rel="noopener noreferrer">
            <span className="n">↓</span>Downloads
          </a>
          <a href="https://github.com/asish231/DoshLang-SNC" target="_blank" rel="noopener noreferrer">
            <span className="n">★</span>GitHub
          </a>
          <a href="https://github.com/asish231/DoshLang-SNC/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">
            <span className="n">§</span>Apache 2.0
          </a>
        </aside>
        <main className="content">
          {body}
          <footer className="site">
            <div className="footer-left">
              <div className="footer-title">
                <b>SNlang</b> — A small typed language that compiles to native code via LLVM &amp; Clang.
              </div>
              <div className="footer-meta">
                <span>© 2026 Asish Sharma / SafarNow</span>
                <span className="dot">·</span>
                <span>Licensed under the <a href="https://github.com/asish231/DoshLang-SNC/blob/main/LICENSE" target="_blank" rel="noopener noreferrer">Apache License 2.0</a></span>
                <span className="dot">·</span>
                <span>All snippets compiler-verified</span>
              </div>
            </div>
            <span className="spacer" />
            <div className="footer-links">
              <a href="https://github.com/asish231/DoshLang-SNC/releases" target="_blank" rel="noopener noreferrer" className="footer-btn">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
                  <polyline points="7 10 12 15 17 10" />
                  <line x1="12" y1="15" x2="12" y2="3" />
                </svg>
                Download Releases
              </a>
              <a href="https://github.com/asish231/DoshLang-SNC" target="_blank" rel="noopener noreferrer" className="footer-btn github">
                <svg width="15" height="15" viewBox="0 0 19 19" fill="currentColor">
                  <use href="/icons.svg#github-icon" />
                </svg>
                GitHub Repo
              </a>
            </div>
          </footer>
        </main>
      </div>
    </div>
  );
}

function SideLink({ to, active, label, num }) {
  return (
    <a href={to} className={active ? 'active' : ''} onClick={(e) => { e.preventDefault(); nav(to); }}>
      {num && <span className="n">{num}</span>}
      {label}
    </a>
  );
}
