import { useCallback, useState } from 'react';
import { Route, Routes } from 'react-router-dom';
import CommandPalette, { usePaletteShortcut } from './app/CommandPalette';
import StatusBar from './app/StatusBar';
import TopBar from './app/TopBar';
import { EmptyState } from './components/ui/state';
import { Toaster } from './components/ui/toaster';
import { TooltipProvider } from './components/ui/tooltip';
import { PageBody, PageHeader } from './components/ui/page';
import ProcessPage from './control/ProcessPage';
import SuiteComparePage from './control/SuiteComparePage';
import SuitePage from './control/SuitePage';
import SuitesPage from './control/SuitesPage';
import Compare from './pages/Compare';
import Run from './pages/Run';
import Runs from './pages/Runs';
import Team from './pages/Team';

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
