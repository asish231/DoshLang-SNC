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
        </aside>
        <main className="content">
          {body}
          <footer className="site">
            <span>SNlang starter &amp; docs — every snippet compiler-verified.</span>
            <span className="spacer" />
            <span>Guide text: docs/LANGUAGE.md · Full status: SNLANG_SPEC.md</span>
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
