import { useNavigate } from "@tanstack/react-router";
import { ChevronDown, ChevronUp, Eye, Search } from "lucide-react";
import { useState } from "react";
import { StatusBadge } from "@/components/machine/status-badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "@/components/ui/table";
import type { Machine } from "@/types/machine";

interface MachineTableProps {
	machines: Machine[];
}

type SortField = keyof Machine;
type SortDirection = "asc" | "desc";

export function MachineTable({ machines }: MachineTableProps) {
	const navigate = useNavigate();
	const [searchTerm, setSearchTerm] = useState("");
	const [sortField, setSortField] = useState<SortField>("name");
	const [sortDirection, setSortDirection] = useState<SortDirection>("asc");

	const handleSort = (field: SortField) => {
		if (sortField === field) {
			setSortDirection(sortDirection === "asc" ? "desc" : "asc");
		} else {
			setSortField(field);
			setSortDirection("asc");
		}
	};

	const getSortIcon = (field: SortField) => {
		if (sortField !== field) return null;
		return sortDirection === "asc" ? (
			<ChevronUp className="w-4 h-4 ml-1" />
		) : (
			<ChevronDown className="w-4 h-4 ml-1" />
		);
	};

	const filteredAndSortedMachines = machines
		.filter(
			(machine) =>
				machine.name.toLowerCase().includes(searchTerm.toLowerCase()) ||
				machine.namespace.toLowerCase().includes(searchTerm.toLowerCase()) ||
				machine.status.toLowerCase().includes(searchTerm.toLowerCase()),
		)
		.sort((a, b) => {
			const aValue = a[sortField];
			const bValue = b[sortField];
			const modifier = sortDirection === "asc" ? 1 : -1;

			if (typeof aValue === "string" && typeof bValue === "string") {
				return aValue.localeCompare(bValue) * modifier;
			}
			return (aValue < bValue ? -1 : aValue > bValue ? 1 : 0) * modifier;
		});

	const handleViewMachine = (machineId: string) => {
		console.log("Navigating to machine detail page with ID:", machineId);
		navigate({ to: "/machines/$machineId", params: { machineId } });
	};

	return (
		<Card className="bg-gradient-card border-0 shadow-card py-0">
			<CardHeader>
				<div className="flex items-center justify-between">
					<div>
						<h1 className="text-3xl font-bold text-foreground">
							Machine Management
						</h1>
						<p className="text-muted-foreground">
							Monitor and manage your containerized workloads
						</p>
					</div>
					<div className="flex items-center space-x-4">
						<div className="relative">
							<Search className="absolute left-3 top-1/2 transform -translate-y-1/2 text-muted-foreground w-4 h-4" />
							<Input
								placeholder="Search machines..."
								value={searchTerm}
								onChange={(e) => setSearchTerm(e.target.value)}
								className="pl-10 w-64 bg-background/50 border-border"
							/>
						</div>
					</div>
				</div>
			</CardHeader>
			<CardContent>
				<div className="rounded-lg border border-border overflow-hidden">
					<Table>
						<TableHeader>
							<TableRow className="border-border hover:bg-muted/50">
								<TableHead
									className="cursor-pointer select-none font-semibold text-foreground"
									onClick={() => handleSort("name")}
								>
									<div className="flex items-center">
										NAME
										{getSortIcon("name")}
									</div>
								</TableHead>
								<TableHead
									className="cursor-pointer select-none font-semibold text-foreground"
									onClick={() => handleSort("namespace")}
								>
									<div className="flex items-center">
										NAMESPACE
										{getSortIcon("namespace")}
									</div>
								</TableHead>
								<TableHead
									className="cursor-pointer select-none font-semibold text-foreground"
									onClick={() => handleSort("mode")}
								>
									<div className="flex items-center">
										MODE
										{getSortIcon("mode")}
									</div>
								</TableHead>
								<TableHead
									className="cursor-pointer select-none font-semibold text-foreground"
									onClick={() => handleSort("status")}
								>
									<div className="flex items-center">
										STATUS
										{getSortIcon("status")}
									</div>
								</TableHead>
								<TableHead className="font-semibold text-foreground">
									IMAGE
								</TableHead>
								<TableHead
									className="cursor-pointer select-none font-semibold text-foreground"
									onClick={() => handleSort("cpus")}
								>
									<div className="flex items-center">
										CPUS
										{getSortIcon("cpus")}
									</div>
								</TableHead>
								<TableHead className="font-semibold text-foreground">
									MEMORY
								</TableHead>
								<TableHead className="font-semibold text-foreground">
									LAST BOOT TIME
								</TableHead>
								<TableHead className="font-semibold text-foreground w-20">
									ACTIONS
								</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{filteredAndSortedMachines.map((machine) => (
								<TableRow
									key={machine.id}
									className="border-border hover:bg-muted/30 transition-colors cursor-pointer"
									onClick={() => handleViewMachine(machine.id)}
								>
									<TableCell className="font-medium text-foreground">
										{machine.name}
									</TableCell>
									<TableCell className="text-muted-foreground">
										{machine.namespace}
									</TableCell>
									<TableCell>
										<span className="px-2 py-1 bg-secondary/50 text-secondary-foreground rounded text-xs font-medium">
											{machine.mode}
										</span>
									</TableCell>
									<TableCell>
										<StatusBadge status={machine.status} />
									</TableCell>
									<TableCell
										className="text-muted-foreground max-w-xs truncate"
										title={machine.image}
									>
										{machine.image}
									</TableCell>
									<TableCell className="text-foreground">
										{machine.cpus}
									</TableCell>
									<TableCell className="text-foreground">
										{machine.memory}
									</TableCell>
									<TableCell className="text-muted-foreground">
										{machine.lastBootTime}
									</TableCell>
									<TableCell>
										<Button
											variant="ghost"
											size="sm"
											onClick={(e) => {
												e.stopPropagation();
												handleViewMachine(machine.id);
											}}
											className="hover:bg-primary/10 hover:text-primary"
										>
											<Eye className="w-4 h-4" />
										</Button>
									</TableCell>
								</TableRow>
							))}
						</TableBody>
					</Table>
				</div>

				{filteredAndSortedMachines.length === 0 && (
					<div className="text-center py-8 text-muted-foreground">
						No machines found matching your search criteria.
					</div>
				)}
			</CardContent>
		</Card>
	);
}
