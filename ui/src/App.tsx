import { NavLink, Route, Routes } from 'react-router-dom';
import { diskLine, modesLine, useHealth } from './components';
import Compare from './pages/Compare';
import Run from './pages/Run';
import Runs from './pages/Runs';
import Team from './pages/Team';

function Placeholder({ title }: { title: string }) {
  return (
    <>
      <header className="page-head"><h1>{title}</h1></header>
      <main className="content"><p className="muted">This page is coming next.</p></main>
    </>
  );
}

export default function App() {
  const h = useHealth();
  return (
    <div className="app">
      <div className="topbar">
        <span className="wordmark">Arena</span>
        <nav className="nav">
          <NavLink to="/" end>Runs</NavLink>
          <NavLink to="/team">Team</NavLink>
        </nav>
        <span className="spacer" />
        <span
          className={`modes${h?.disk && h.disk.bytes >= h.disk.limit_bytes ? ' disk-full' : ''}`}
          title="Space used by saved worlds, and the limit above which new runs are refused"
        >
          {diskLine(h)}
        </span>
        <span className="modes" title="Agent modes available on this machine">{modesLine(h)}</span>
      </div>
      <Routes>
        <Route path="/" element={<Runs />} />
        <Route path="/team" element={<Team />} />
        <Route path="/runs/:runId/:branchId" element={<Run />} />
        <Route path="/compare/:a/:b" element={<Compare />} />
        <Route path="*" element={<Placeholder title="Not found" />} />
      </Routes>
    </div>
  );
}
