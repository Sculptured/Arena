import { NavLink, Route, Routes } from 'react-router-dom';
import { diskLine, modesLine, useHealth } from './components';
import { ProviderStatusLine } from './ProviderFields';
import { bestProviders, checkProviders, useCheckedProviders } from './providers';
import Compare from './pages/Compare';
import Run from './pages/Run';
import Runs from './pages/Runs';
import Team from './pages/Team';
import ProcessPage from './control/ProcessPage';
import SuiteComparePage from './control/SuiteComparePage';
import SuitePage from './control/SuitePage';
import SuitesPage from './control/SuitesPage';

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
  const checked = useCheckedProviders();
  const statuses = bestProviders(checked.list, h?.providers);
  return (
    <div className="app">
      <div className="topbar">
        <span className="wordmark">Arena</span>
        <nav className="nav">
          <NavLink to="/" end>Runs</NavLink>
          <NavLink to="/team">Team</NavLink>
          <NavLink to="/suites">Suites</NavLink>
        </nav>
        <span className="spacer" />
        <span
          className={`modes${h?.disk && h.disk.bytes >= h.disk.limit_bytes ? ' disk-full' : ''}`}
          title="Space used by saved worlds, and the limit above which new runs are refused"
        >
          {diskLine(h)}
        </span>
      </div>
      {h && (
        <div className="status-line">
          <span className="modes" title="Agent CLIs found on this machine">{modesLine(h)}</span>
          <span className="label">API mode</span>
          <ProviderStatusLine
            statuses={statuses}
            loading={checked.loading}
            error={checked.error}
            onCheck={() => { checkProviders().catch(() => {}); }}
          />
        </div>
      )}
      <Routes>
        <Route path="/" element={<Runs />} />
        <Route path="/team" element={<Team />} />
        <Route path="/runs/:runId/:branchId" element={<Run />} />
        <Route path="/compare/:a/:b" element={<Compare />} />
        <Route path="/suites" element={<SuitesPage />} />
        <Route path="/suites/compare/:a/:b" element={<SuiteComparePage />} />
        <Route path="/suites/:id" element={<SuitePage />} />
        <Route path="/control/:runId/:branchId" element={<ProcessPage />} />
        <Route path="*" element={<Placeholder title="Not found" />} />
      </Routes>
    </div>
  );
}
