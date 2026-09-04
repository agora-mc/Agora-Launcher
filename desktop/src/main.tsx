// import './i18n'; // commented out: i18n deferred post-v1
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './index.css';
import { ThemeProvider } from '@/components/theme/theme-provider';
import { AdvancedModeProvider } from '@/components/AdvancedModeContext';
import { ControllerProvider } from './features/controller/ControllerProvider';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ControllerProvider>
      <ThemeProvider>
        <AdvancedModeProvider>
          <App />
        </AdvancedModeProvider>
      </ThemeProvider>
    </ControllerProvider>
  </React.StrictMode>,
);
