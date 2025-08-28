import { Separator } from "@radix-ui/react-separator";
import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import { AppSidebar } from "@/components/app-sidebar";
import { MachineTable } from "@/components/machine/machine-table";
import { ModeToggle } from "@/components/mode-toggle";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
} from "@/components/ui/breadcrumb";
import {
	SidebarInset,
	SidebarProvider,
	SidebarTrigger,
} from "@/components/ui/sidebar";

import type { Machine } from "@/types/machine";

export const Route = createFileRoute("/machines")({
	component: Machines,
});

function Machines() {
	// Mock data for machines using the proper types
	const [machines, _setMachiness] = useState<Machine[]>([
		{
			id: "vm-1",
			name: "nginx-web-server",
			namespace: "default",
			mode: "regular",
			status: "ready",
			image: "nginx:latest",
			cpus: 2,
			memory: "1Gi",
			lastBootTime: "312ms",
			createdAt: "2025-07-15T09:15:00Z",
			uptime: "42 days",
		},
		{
			id: "vm-2",
			name: "hono-backend",
			namespace: "default",
			mode: "flash",
			status: "ready",
			image: "hono:latest",
			cpus: 1,
			memory: "512Mi",
			lastBootTime: "110ms",
			createdAt: "2025-08-01T16:20:00Z",
			uptime: "18 days",
		},
		{
			id: "vm-3",
			name: "notifications-queue",
			namespace: "default",
			mode: "regular",
			status: "pending",
			image: "queue:latest",
			cpus: 1,
			memory: "256Mi",
			lastBootTime: "560ms",
			createdAt: "2025-08-23T14:10:00Z",
			uptime: "5 days",
		},
		{
			id: "vm-4",
			name: "database-primary",
			namespace: "production",
			mode: "regular",
			status: "ready",
			image: "postgres:15",
			cpus: 4,
			memory: "4Gi",
			lastBootTime: "410ms",
			createdAt: "2025-05-28T12:00:00Z",
			uptime: "92 days",
		},
		{
			id: "vm-5",
			name: "redis-cache",
			namespace: "production",
			mode: "flash",
			status: "ready",
			image: "redis:alpine",
			cpus: 1,
			memory: "128Mi",
			lastBootTime: "250ms",
			createdAt: "2025-08-16T11:20:00Z",
			uptime: "12 days",
		},
	]);

	return (
		<SidebarProvider>
			<AppSidebar />
			<SidebarInset>
				<header className="justify-between flex h-16 shrink-0 items-center gap-2 transition-[width,height] ease-linear group-has-data-[collapsible=icon]/sidebar-wrapper:h-12">
					<div className="flex items-center gap-2 px-4">
						<SidebarTrigger className="-ml-1" />
						<Separator
							orientation="vertical"
							className="mr-2 data-[orientation=vertical]:h-4"
						/>
						<Breadcrumb>
							<BreadcrumbList>
								<BreadcrumbItem className="hidden md:block">
									<BreadcrumbLink href="#">Machines</BreadcrumbLink>
								</BreadcrumbItem>
							</BreadcrumbList>
						</Breadcrumb>
					</div>
					<div className="px-4">
						<ModeToggle />
					</div>
				</header>
				<MachineTable machines={machines}></MachineTable>
			</SidebarInset>
		</SidebarProvider>
	);
}
