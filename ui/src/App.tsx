import { Suspense, lazy, useCallback, useState } from 'react';
import { Route, Routes } from 'react-router-dom';
import CommandPalette, { usePaletteShortcut } from './app/CommandPalette';
import StatusBar from './app/StatusBar';
import TopBar from './app/TopBar';
import { EmptyState, LoadingState } from './components/ui/state';
import { Toaster } from './components/ui/toaster';
import { TooltipProvider } from './components/ui/tooltip';
import { PageBody, PageHeader } from './components/ui/page';
import Runs from './pages/Runs';

// Pages other than the runs list load on first visit (React Flow and uPlot stay out of the first load).
const ProcessPage = lazy(() => import('./control/ProcessPage'));
const SuiteComparePage = lazy(() => import('./control/SuiteComparePage'));
const SuitePage = lazy(() => import('./control/SuitePage'));
const SuitesPage = lazy(() => import('./control/SuitesPage'));
const Compare = lazy(() => import('./pages/Compare'));
const Run = lazy(() => import('./pages/Run'));
const Team = lazy(() => import('./pages/Team'));

function NotFound() {
  return (
    <>
      <PageHeader title="Not found" />
      <PageBody><EmptyState title="There is no page at this address." hint="Use the menu above, or press ⌘K to go to a run or a suite." /></PageBody>
    </>
  );
}

function AppRoutes() {
  return (
    <Suspense fallback={<PageBody><LoadingState label="Loading the page…" /></PageBody>}>
      <Routes>
        <Route path="/" element={<Runs />} />
        <Route path="/team" element={<Team />} />
        <Route path="/runs/:runId/:branchId" element={<Run />} />
        <Route path="/compare/:a/:b" element={<Compare />} />
        <Route path="/suites" element={<SuitesPage />} />
        <Route path="/suites/compare/:a/:b" element={<SuiteComparePage />} />
        <Route path="/suites/:id" element={<SuitePage />} />
        <Route path="/control/:runId/:branchId" element={<ProcessPage />} />
        <Route path="*" element={<NotFound />} />
      </Routes>
    </Suspense>
  );
}

export default function App() {
  const [palette, setPalette] = useState(false);
  const toggle = useCallback(() => setPalette((o) => !o), []);
  usePaletteShortcut(toggle);
  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex min-h-full flex-col">
        <TopBar onPalette={() => setPalette(true)} />
        <div className="min-w-0 flex-1">
          <AppRoutes />
        </div>
        <StatusBar />
      </div>
      <CommandPalette open={palette} onOpenChange={setPalette} />
      <Toaster />
    </TooltipProvider>
  );
}
