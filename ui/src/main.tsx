import '@fontsource-variable/inter';
import '@fontsource-variable/jetbrains-mono';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import { initTheme } from './app/theme';
import './styles/index.css';

initTheme();

function render() {
  createRoot(document.getElementById('root')!).render(
    <StrictMode>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </StrictMode>,
  );
}

// A build with VITE_ARENA_FIXTURES=1 answers /api from the pharma fixtures (screenshots, design
// review); any other build leaves this branch out entirely.
if (import.meta.env.VITE_ARENA_FIXTURES === '1') import('./dev/fixtureApi').then((m) => { m.install(); render(); });
else render();
