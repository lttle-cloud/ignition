import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Progress } from "@/components/ui/progress";
import {
	TrendingUp,
	TrendingDown,
	Activity,
	Zap,
	HardDrive,
	Network,
} from "lucide-react";

interface MetricCardProps {
	title: string;
	value: string | number;
	percentage?: number;
	trend?: "up" | "down" | "stable";
	icon: React.ReactNode;
	description?: string;
}

function MetricCard({
	title,
	value,
	percentage,
	trend,
	icon,
	description,
}: MetricCardProps) {
	const getTrendIcon = () => {
		if (trend === "up")
			return <TrendingUp className="w-4 h-4 text-status-warning" />;
		if (trend === "down")
			return <TrendingDown className="w-4 h-4 text-status-ready" />;
		return null;
	};

	return (
		<Card className="bg-gradient-card border-border shadow-card">
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium text-muted-foreground">
					{title}
				</CardTitle>
				<div className="flex items-center space-x-2">
					{getTrendIcon()}
					<div className="text-primary">{icon}</div>
				</div>
			</CardHeader>
			<CardContent>
				<div className="text-2xl font-bold text-foreground">{value}</div>
				{percentage !== undefined && (
					<div className="mt-2">
						<Progress value={percentage} className="h-2" />
						<p className="text-xs text-muted-foreground mt-1">
							{percentage}% utilization
						</p>
					</div>
				)}
				{description && (
					<p className="text-xs text-muted-foreground mt-1">{description}</p>
				)}
			</CardContent>
		</Card>
	);
}

interface MetricsCardProps {
	metrics: {
		cpu: { usage: number };
		memory: { percentage: number; used: string; total: string };
		diskIO: { read: string; write: string };
		network: { bytesIn: string; bytesOut: string };
		flashLocks: number;
	};
}

export function MetricsCard({ metrics }: MetricsCardProps) {
	return (
		<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
			<MetricCard
				title="CPU Usage"
				value={`${metrics.cpu.usage}%`}
				percentage={metrics.cpu.usage}
				trend={
					metrics.cpu.usage > 70
						? "up"
						: metrics.cpu.usage < 30
							? "down"
							: "stable"
				}
				icon={<Activity className="w-4 h-4" />}
				description="Current CPU utilization"
			/>

			<MetricCard
				title="Memory Usage"
				value={metrics.memory.used}
				percentage={metrics.memory.percentage}
				trend={metrics.memory.percentage > 80 ? "up" : "stable"}
				icon={<Zap className="w-4 h-4" />}
				description={`${metrics.memory.used} of ${metrics.memory.total}`}
			/>

			<MetricCard
				title="Disk I/O"
				value={metrics.diskIO.read}
				trend="stable"
				icon={<HardDrive className="w-4 h-4" />}
				description={`Read: ${metrics.diskIO.read}, Write: ${metrics.diskIO.write}`}
			/>

			<MetricCard
				title="Network In"
				value={metrics.network.bytesIn}
				trend="stable"
				icon={<Network className="w-4 h-4" />}
				description="Incoming network traffic"
			/>

			<MetricCard
				title="Network Out"
				value={metrics.network.bytesOut}
				trend="stable"
				icon={<Network className="w-4 h-4" />}
				description="Outgoing network traffic"
			/>

			<MetricCard
				title="Flash Locks"
				value={metrics.flashLocks}
				trend={metrics.flashLocks > 3 ? "up" : "stable"}
				icon={<Zap className="w-4 h-4" />}
				description="Active flash memory locks"
			/>
		</div>
	);
}
