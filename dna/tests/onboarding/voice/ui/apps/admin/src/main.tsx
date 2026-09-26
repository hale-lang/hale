import '@hale/tokens';
import '@hale/tokens/fonts.css';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { App } from './App';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The admin event stream will invalidate what it names; until it is
      // wired, a quiet refetch keeps the instrument roughly current.
      staleTime: 5_000,
      refetchInterval: 15_000,
      retry: 1,
    },
  },
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
);
