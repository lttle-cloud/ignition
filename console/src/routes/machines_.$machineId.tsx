import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { ResourcesCard } from "@/components/machine/resources-card";
import { StatusBadge } from "@/components/machine/status-badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Badge } from "@/components/ui/badge";
import { ArrowLeft, Server, Clock, Cpu, MemoryStick } from "lucide-react";
import { getMachineDetails } from "@/data/mockData";
import { MetricsCard } from "@/components/machine/metrics-card";
import { LogsViewer } from "@/components/machine/logs-viewer";
import { AppSidebar } from "@/components/app-sidebar";
import {
	SidebarProvider,
	SidebarInset,
	SidebarTrigger,
} from "@/components/ui/sidebar";
import { Separator } from "@radix-ui/react-separator";
import {
	BreadcrumbLink,
	Breadcrumb,
	BreadcrumbList,
	BreadcrumbItem,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";

export const Route = createFileRoute("/machines_/$machineId")({
	component: MachineDetail,
	loader: ({ params: { machineId } }) => getMachineDetails(machineId),
});

export default function MachineDetail() {
	const navigate = useNavigate();
	const machineDetails = Route.useLoaderData();

	if (!machineDetails) {
		return (
			<div className="min-h-screen bg-gradient-subtle p-6 flex items-center justify-center">
				<div className="text-center">
					<h1 className="text-2xl font-bold text-foreground mb-4">
						Machine Not Found
					</h1>
					<p className="text-muted-foreground mb-6">
						The requested machine could not be found.
					</p>
					<Button onClick={() => navigate({ to: "/" })} variant="outline">
						<ArrowLeft className="w-4 h-4 mr-2" />
						Back to Machines
					</Button>
				</div>
			</div>
		);
	}

	const { machine, metrics, resources, logs } = machineDetails;

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
									<BreadcrumbLink>
										<Link to="/machines"> Machines</Link>
									</BreadcrumbLink>
								</BreadcrumbItem>
								<BreadcrumbSeparator className="hidden md:block" />
								<BreadcrumbItem>
									<BreadcrumbPage>{machine.name}</BreadcrumbPage>
								</BreadcrumbItem>
							</BreadcrumbList>
						</Breadcrumb>
					</div>
				</header>
				<div className="min-h-screen bg-gradient-subtle p-6">
					<div className="max-w-7xl mx-auto space-y-6">
						{/* Header */}
						<div className="flex items-center justify-between">
							<div className="flex items-center space-x-4">
								<div className="flex items-center space-x-3">
									<div className="p-2 bg-primary/20 rounded-lg">
										<Server className="w-8 h-8 text-primary" />
									</div>
									<div>
										<div className="flex items-center space-x-3">
											<h1 className="text-3xl font-bold text-foreground">
												{machine.name}
											</h1>
											<StatusBadge status={machine.status} />
										</div>
										<p className="text-muted-foreground">
											{machine.namespace} namespace
										</p>
									</div>
								</div>
							</div>
						</div>

						{/* Machine Overview */}
						<Card className="bg-gradient-card border-border shadow-card">
							<CardHeader>
								<CardTitle className="text-xl font-bold">
									Machine Overview
								</CardTitle>
							</CardHeader>
							<CardContent>
								<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
									<div className="space-y-2">
										<div className="flex items-center text-muted-foreground">
											<Server className="w-4 h-4 mr-2" />
											<span className="text-sm">Mode</span>
										</div>
										<Badge variant="outline" className="bg-secondary/50">
											{machine.mode}
										</Badge>
									</div>

									<div className="space-y-2">
										<div className="flex items-center text-muted-foreground">
											<Cpu className="w-4 h-4 mr-2" />
											<span className="text-sm">CPU Cores</span>
										</div>
										<p className="text-lg font-semibold text-foreground">
											{machine.cpus}
										</p>
									</div>

									<div className="space-y-2">
										<div className="flex items-center text-muted-foreground">
											<MemoryStick className="w-4 h-4 mr-2" />
											<span className="text-sm">Memory</span>
										</div>
										<p className="text-lg font-semibold text-foreground">
											{machine.memory}
										</p>
									</div>

									<div className="space-y-2">
										<div className="flex items-center text-muted-foreground">
											<Clock className="w-4 h-4 mr-2" />
											<span className="text-sm">Uptime</span>
										</div>
										<p className="text-lg font-semibold text-foreground">
											{machine.uptime}
										</p>
									</div>
								</div>

								<div className="mt-6 pt-6 border-t border-border">
									<div className="space-y-2">
										<div className="flex items-center text-muted-foreground">
											<span className="text-sm">Container Image</span>
										</div>
										<p className="text-sm font-mono text-foreground bg-background/50 p-2 rounded border">
											{machine.image}
										</p>
									</div>
								</div>
							</CardContent>
						</Card>

						{/* Metrics */}
						<div className="space-y-4">
							<h2 className="text-2xl font-bold text-foreground">
								System Metrics
							</h2>
							<MetricsCard metrics={metrics} />
						</div>

						{/* Tabs for detailed information */}
						<Tabs defaultValue="logs" className="space-y-6">
							<TabsList className="bg-card border border-border">
								<TabsTrigger
									value="logs"
									className="data-[state=active]:bg-primary data-[state=active]:text-primary-foreground"
								>
									Logs
								</TabsTrigger>
								<TabsTrigger
									value="resources"
									className="data-[state=active]:bg-primary data-[state=active]:text-primary-foreground"
								>
									Resources
								</TabsTrigger>
							</TabsList>

							<TabsContent value="logs">
								<LogsViewer logs={logs} />
							</TabsContent>

							<TabsContent value="resources">
								<div className="space-y-4">
									<h3 className="text-xl font-bold text-foreground">
										Related Resources
									</h3>
									<ResourcesCard resources={resources} />
								</div>
							</TabsContent>
						</Tabs>
					</div>
				</div>
			</SidebarInset>
		</SidebarProvider>
	);
}
