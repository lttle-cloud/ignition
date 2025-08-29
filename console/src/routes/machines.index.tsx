import { createFileRoute } from "@tanstack/react-router";
import { MachineTable } from "@/components/machine/machine-table";

import { mockMachines } from "@/data/mockData";

export const Route = createFileRoute("/machines/")({
	component: MachinesIndex,
});

function MachinesIndex() {
	return <MachineTable machines={mockMachines}></MachineTable>;
}
