import { createFileRoute, Outlet } from "@tanstack/react-router";

export const Route = createFileRoute("/machines")({
	component: MachinesRoute,
	loader: () => {
		return { crumb: "Machines" };
	},
});

function MachinesRoute() {
	return <Outlet />;
}
