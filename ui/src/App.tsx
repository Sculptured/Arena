import { Suspense, lazy, useCallback, useEffect, useState } from 'react';
import { Navigate, Route, Routes, useParams } from 'react-router-dom';
import CommandPalette, { usePaletteShortcut } from './app/CommandPalette';
import { type Mode, PALETTE_EVENT } from './app/palette';
import StatusBar from './app/StatusBar';
import TopBar from './app/TopBar';
import { EmptyState } from './components/ui/state';
import { Toaster } from './components/ui/toaster';
import { TooltipProvider } from './components/ui/tooltip';
import { PageBody, PageHeader } from './components/ui/page';
import ExperimentsPage from './experiments/ExperimentsPage';

// Pages other than Experiments load on first visit (React Flow and uPlot stay out of the first load).
const ExperimentPage = lazy(() => import('./experiments/ExperimentPage'));
const ExperimentsCompare = lazy(() => import('./experiments/ExperimentPage').then((m) => ({ default: m.ComparePage })));
const NewExperimentPage = lazy(() => import('./experiments/NewExperimentPage'));
const Home = lazy(() => import('./pages/Home'));
const ProcessPage = lazy(() => import('./control/ProcessPage'));
const Compare = lazy(() => import('./pages/Compare'));
const ConnectPage = lazy(() => import('./connect/ConnectPage'));
const Run = lazy(() => import('./pages/Run'));
const Runs = lazy(() => import('./pages/Runs'));
const Team = lazy(() => import('./pages/Team'));
const World = lazy(() => import('./pages/World'));

function NotFound() {
  return (
    <>
      <PageHeader title="Not found" />
      <PageBody><EmptyState title="There is no page at this address." hint="Press ⌘K to go to an experiment or a run." /></PageBody>
    </>
  );
}

/** An old address (suites became experiments), sent on to its new one. */
function Moved({ to }: { to: (p: Record<string, string | undefined>) => string }) {
  return <Navigate to={to(useParams())} replace />;
}

function AppRoutes() {
  return (
    <Suspense fallback={null}>
      <Routes>
        <Route path="/" element={<ExperimentsPage />} />
        <Route path="/experiments/new" element={<NewExperimentPage />} />
        <Route path="/experiments/compare" element={<ExperimentsCompare />} />
        <Route path="/experiments/:id" element={<ExperimentPage />} />
        <Route path="/run" element={<Home />} />
        <Route path="/runs" element={<Runs />} />
        <Route path="/runs/:runId/:branchId" element={<Run />} />
        <Route path="/team" element={<Team />} />
        <Route path="/connect" element={<ConnectPage />} />
        <Route path="/compare/:a/:b" element={<Compare />} />
        <Route path="/history" element={<Navigate to="/runs" replace />} />
        <Route path="/suites" element={<Navigate to="/" replace />} />
        <Route path="/suites/compare/:a/:b" element={<Moved to={(p) => `/experiments/compare?ids=${p.a},${p.b}`} />} />
        <Route path="/suites/:id" element={<Moved to={(p) => `/experiments/${p.id}`} />} />
        <Route path="/control/:runId/:branchId" element={<ProcessPage />} />
        <Route path="/world/:runId/:branchId" element={<World />} />
        <Route path="*" element={<NotFound />} />
      </Routes>
    </Suspense>
  );
}

export default function App() {
  const [palette, setPalette] = useState(false);
  const [start, setStart] = useState<Mode | undefined>(undefined);
  const toggle = useCallback(() => { setStart(undefined); setPalette((o) => !o); }, []);
  usePaletteShortcut(toggle);
  useEffect(() => {
    const open = (e: Event) => { setStart((e as CustomEvent<Mode>).detail); setPalette(true); };
    window.addEventListener(PALETTE_EVENT, open);
    return () => window.removeEventListener(PALETTE_EVENT, open);
  }, []);
  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex min-h-full flex-col">
        <TopBar onPalette={(m) => { setStart(m); setPalette(true); }} />
        <div className="min-w-0 flex-1">
          <AppRoutes />
        </div>
        <StatusBar />
      </div>
      <CommandPalette open={palette} onOpenChange={setPalette} start={start} />
      <Toaster />
    </TooltipProvider>
  );
}
