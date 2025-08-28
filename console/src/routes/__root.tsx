import { TanstackDevtools } from "@tanstack/react-devtools";
import { createRootRoute, Outlet } from "@tanstack/react-router";
import { TanStackRouterDevtoolsPanel } from "@tanstack/react-router-devtools";
import { ThemeProvider } from "@/components/theme-provider";

export const Route = createRootRoute({
	component: () => (
		<>
			<ThemeProvider defaultTheme="dark" storageKey="vite-ui-theme">
				<Outlet />
				<TanstackDevtools
					config={{
						position: "bottom-left",
					}}
					plugins={[
						{
							name: "Tanstack Router",
							render: <TanStackRouterDevtoolsPanel />,
						},
					]}
				/>
			</ThemeProvider>
		</>
	),
});
