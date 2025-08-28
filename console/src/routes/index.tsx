import { Separator } from "@radix-ui/react-separator";
import { createFileRoute } from "@tanstack/react-router";
import { Activity, Clock, Globe, Pause, Play } from "lucide-react";
import { AppSidebar } from "@/components/app-sidebar";
import { ModeToggle } from "@/components/mode-toggle";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
} from "@/components/ui/breadcrumb";
import { Button } from "@/components/ui/button";
import {
	SidebarInset,
	SidebarProvider,
	SidebarTrigger,
} from "@/components/ui/sidebar";
import { cn } from "@/lib/utils";

export const Route = createFileRoute("/")({
	component: Applications,
});

function Applications() {
	// Mock data for applications
	const applications = [
		{
			id: "app-1",
			name: "API Gateway",
			status: "running",
			uptime: "42 days",
			cpu: 24,
			memory: 42,
			requests: "1.2k/s",
			errors: 0,
		},
		{
			id: "app-2",
			name: "User Service",
			status: "running",
			uptime: "18 days",
			cpu: 12,
			memory: 28,
			requests: "850/s",
			errors: 2,
		},
		{
			id: "app-3",
			name: "Payment Processor",
			status: "paused",
			uptime: "5 days",
			cpu: 0,
			memory: 5,
			requests: "0/s",
			errors: 0,
		},
	];

	// Mock metrics data
	const metrics = [
		{ name: "Requests", value: "12.4k", change: "+12%" },
		{ name: "Errors", value: "18", change: "-3%" },
		{ name: "Latency", value: "42ms", change: "-5%" },
		{ name: "Uptime", value: "99.98%", change: "+0.1%" },
	];

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
									<BreadcrumbLink href="#">Applications</BreadcrumbLink>
								</BreadcrumbItem>
							</BreadcrumbList>
						</Breadcrumb>
					</div>
					<div className="px-4">
						<ModeToggle />
					</div>
				</header>
				<div className="flex flex-1 flex-col gap-4 p-4 pt-0">
					{/* Metrics Overview */}
					<div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
						{metrics.map((metric) => (
							<div key={metric.name} className="bg-muted/50 rounded-lg p-4">
								<p className="text-sm font-medium text-muted-foreground">
									{metric.name}
								</p>
								<div className="flex items-baseline gap-2">
									<p className="text-2xl font-bold text-foreground">
										{metric.value}
									</p>
									<p
										className={cn("text-xs font-medium", {
											"text-green-3": metric.change.startsWith("+"),
											"text-red-3": metric.change.startsWith("-"),
											"text-blue-2":
												!metric.change.startsWith("+") &&
												!metric.change.startsWith("-"),
										})}
									>
										{metric.change}
									</p>
								</div>
							</div>
						))}
					</div>

					{/* Application Cards */}
					<div className="grid auto-rows-min gap-4 md:grid-cols-3">
						{applications.map((app) => (
							<div key={app.id} className="bg-muted/50 rounded-xl p-6">
								<div className="flex justify-between items-start">
									<div>
										<h3 className="font-semibold text-lg">{app.name}</h3>
										<div className="flex items-center gap-2 mt-1">
											<div
												className={cn(
													"w-2 h-2 rounded-full",
													app.status === "running"
														? "bg-green-3"
														: "bg-yellow-3",
												)}
											></div>
											<span className="text-sm capitalize">{app.status}</span>
											<span className="text-sm text-muted-foreground">•</span>
											<span className="text-sm text-muted-foreground">
												{app.uptime}
											</span>
										</div>
									</div>
									<Button variant="ghost" size="icon">
										{app.status === "running" ? (
											<Pause className="h-4 w-4" />
										) : (
											<Play className="h-4 w-4" />
										)}
									</Button>
								</div>

								<div className="mt-4 space-y-3">
									<div className="flex justify-between text-sm">
										<span className="text-muted-foreground">CPU</span>
										<span>{app.cpu}%</span>
									</div>
									<div className="w-full bg-secondary rounded-full h-2">
										<div
											className={cn(
												"h-2 rounded-full",
												app.cpu > 80
													? "bg-red-2"
													: app.memory > 50
														? "bg-yellow-2"
														: "bg-blue-2",
											)}
											style={{ width: `${app.cpu}%` }}
										></div>
									</div>

									<div className="flex justify-between text-sm">
										<span className="text-muted-foreground">Memory</span>
										<span>{app.memory}%</span>
									</div>
									<div className="w-full bg-secondary rounded-full h-2">
										<div
											className={cn(
												"h-2 rounded-full",
												app.memory > 80
													? "bg-red-2"
													: app.memory > 50
														? "bg-yellow-2"
														: "bg-blue-2",
											)}
											style={{ width: `${app.memory}%` }}
										></div>
									</div>

									<div className="flex justify-between text-sm">
										<div className="flex items-center gap-1">
											<Activity className="h-3 w-3" />
											<span className="text-muted-foreground">Requests</span>
										</div>
										<span>{app.requests}</span>
									</div>

									<div className="flex justify-between text-sm">
										<div className="flex items-center gap-1">
											<Clock className="h-3 w-3" />
											<span className="text-muted-foreground">Errors</span>
										</div>
										<span>{app.errors}</span>
									</div>
								</div>
							</div>
						))}
					</div>

					{/* Resource Usage Graph */}
					<div className="bg-muted/50 rounded-xl p-6">
						<div className="flex justify-between items-center mb-4">
							<h3 className="font-semibold text-lg">
								Resource Usage by Application
							</h3>
							<div className="flex gap-2">
								<Button variant="outline" size="sm">
									<Globe className="h-4 w-4 mr-2" />
									All Services
								</Button>
							</div>
						</div>

						{/* Application Resource Usage Chart */}
						<div className="h-64 flex items-end justify-between gap-2 mb-4">
							{/* Mock data for each application with different colors */}
							{[
								{
									name: "API Gateway",
									data: [45, 52, 48, 65, 78, 82, 95, 120, 145, 160, 180, 175],
									colorClass: "bg-pink-2",
								},
								{
									name: "User Service",
									data: [30, 35, 40, 45, 50, 55, 60, 65, 70, 75, 80, 85],
									colorClass: "bg-teal-2",
								},
								{
									name: "Payment Processor",
									data: [15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 65, 70],
									colorClass: "bg-blue-2",
								},
							].map((app, appIndex) => (
								<div key={app.name} className="flex flex-col flex-1 h-full">
									<div className="text-xs text-muted-foreground mb-1 text-center">
										{app.name}
									</div>
									<div className="flex items-end justify-center gap-1 h-full">
										{app.data.map((value) => (
											<div
												key={`${appIndex}-${value}`}
												className={cn(
													"w-full rounded-t transition-all hover:opacity-90",
													app.colorClass,
												)}
												style={{
													height: `${(value / 200) * 100}%`,
												}}
												title={`${app.name}: ${value}%`}
											></div>
										))}
									</div>
								</div>
							))}
						</div>

						{/* Chart Legend */}
						<div className="flex flex-wrap gap-4 mt-4">
							<div className="flex items-center gap-2">
								<div className="w-3 h-3 rounded-full bg-pink-2"></div>
								<span className="text-sm text-muted-foreground">
									API Gateway
								</span>
							</div>
							<div className="flex items-center gap-2">
								<div className="w-3 h-3 rounded-full bg-teal-2"></div>
								<span className="text-sm text-muted-foreground">
									User Service
								</span>
							</div>
							<div className="flex items-center gap-2">
								<div className="w-3 h-3 rounded-full bg-blue-2"></div>
								<span className="text-sm text-muted-foreground">
									Payment Processor
								</span>
							</div>
						</div>
					</div>
				</div>
			</SidebarInset>
		</SidebarProvider>
	);
}
