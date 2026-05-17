import { Outlet, createRootRoute } from '@tanstack/react-router'
import { TanStackRouterDevtoolsPanel } from '@tanstack/react-router-devtools'
import { TanStackDevtools } from '@tanstack/react-devtools'

import {
  TakeoverOverlay,
  UserControlIndicator,
} from '@/components/ui/TakeoverOverlay'

// Root route for the SPA. We dropped the SSR `shellComponent` form
// (which owns the entire <html>/<body> tree) because the app ships as
// a pure client-side bundle now — head metadata lives in
// `apps/web/index.html`, and `main.tsx` is the actual mount point.
// Keeping just an <Outlet/> here means every child route inherits a
// minimal wrapper without re-rendering the whole document chrome.

export const Route = createRootRoute({
  component: RootDocument,
})

function RootDocument() {
  return (
    <>
      <Outlet />
      {/* Agent-takeover overlay — the only global notification
          surface. Toasts were removed when this landed; the overlay
          covers all the "something is happening in the background"
          cases and inline UI handles direct-action confirmations. */}
      <TakeoverOverlay />
      <UserControlIndicator />
      {import.meta.env.DEV && (
        <TanStackDevtools
          config={{ position: 'bottom-right' }}
          plugins={[
            {
              name: 'Tanstack Router',
              render: <TanStackRouterDevtoolsPanel />,
            },
          ]}
        />
      )}
    </>
  )
}
