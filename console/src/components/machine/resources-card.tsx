/** biome-ignore-all lint/suspicious/noArrayIndexKey: <explanation> */
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { StatusBadge } from "@/components/machine/status-badge";
import {
	HardDrive,
	Globe,
	Link,
	Database,
	Server,
	Settings,
} from "lucide-react";

interface ResourcesCardProps {
	resources: {
		boundVolumes: Array<{
			name: string;
			mountPath: string;
			size: string;
			type: "persistent" | "ephemeral";
		}>;
		boundServices: Array<{
			name: string;
			port: number;
			protocol: "HTTP" | "HTTPS" | "TCP" | "UDP";
			status: "healthy" | "unhealthy";
		}>;
		dependsOn: Array<{
			name: string;
			type: "machine" | "service" | "volume";
			status: "ready" | "pending" | "error";
		}>;
	};
}

export function ResourcesCard({ resources }: ResourcesCardProps) {
	const getTypeIcon = (type: string) => {
		switch (type) {
			case "machine":
				return <Server className="w-4 h-4" />;
			case "service":
				return <Globe className="w-4 h-4" />;
			case "volume":
				return <Database className="w-4 h-4" />;
			default:
				return <Settings className="w-4 h-4" />;
		}
	};

	return (
		<div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
			{/* Bound Volumes */}
			<Card className="bg-gradient-card border-border shadow-card">
				<CardHeader>
					<CardTitle className="text-lg font-semibold flex items-center">
						<HardDrive className="w-5 h-5 mr-2 text-primary" />
						Bound Volumes
					</CardTitle>
				</CardHeader>
				<CardContent>
					<div className="space-y-3">
						{resources.boundVolumes.map((volume, index) => (
							<div
								key={index}
								className="p-3 rounded-lg bg-background/50 border border-border"
							>
								<div className="flex items-center justify-between mb-2">
									<h4 className="font-medium text-foreground">{volume.name}</h4>
									<Badge
										variant="outline"
										className={
											volume.type === "persistent"
												? "bg-primary/20 text-primary border-primary/30"
												: "bg-muted/50 text-muted-foreground border-muted"
										}
									>
										{volume.type}
									</Badge>
								</div>
								<p className="text-sm text-muted-foreground mb-1">
									<span className="font-mono">{volume.mountPath}</span>
								</p>
								<p className="text-sm text-muted-foreground">
									Size: {volume.size}
								</p>
							</div>
						))}
						{resources.boundVolumes.length === 0 && (
							<p className="text-sm text-muted-foreground text-center py-4">
								No bound volumes
							</p>
						)}
					</div>
				</CardContent>
			</Card>

			{/* Bound Services */}
			<Card className="bg-gradient-card border-border shadow-card">
				<CardHeader>
					<CardTitle className="text-lg font-semibold flex items-center">
						<Globe className="w-5 h-5 mr-2 text-primary" />
						Bound Services
					</CardTitle>
				</CardHeader>
				<CardContent>
					<div className="space-y-3">
						{resources.boundServices.map((service, index) => (
							<div
								key={index}
								className="p-3 rounded-lg bg-background/50 border border-border"
							>
								<div className="flex items-center justify-between mb-2">
									<h4 className="font-medium text-foreground">
										{service.name}
									</h4>
									<StatusBadge
										status={service.status === "healthy" ? "ready" : "error"}
									/>
								</div>
								<div className="flex items-center justify-between text-sm text-muted-foreground">
									<span>Port: {service.port}</span>
									<Badge variant="outline" className="bg-secondary/50">
										{service.protocol}
									</Badge>
								</div>
							</div>
						))}
						{resources.boundServices.length === 0 && (
							<p className="text-sm text-muted-foreground text-center py-4">
								No bound services
							</p>
						)}
					</div>
				</CardContent>
			</Card>

			{/* Dependencies */}
			<Card className="bg-gradient-card border-border shadow-card">
				<CardHeader>
					<CardTitle className="text-lg font-semibold flex items-center">
						<Link className="w-5 h-5 mr-2 text-primary" />
						Dependencies
					</CardTitle>
				</CardHeader>
				<CardContent>
					<div className="space-y-3">
						{resources.dependsOn.map((dependency, index) => (
							<div
								key={index}
								className="p-3 rounded-lg bg-background/50 border border-border"
							>
								<div className="flex items-center justify-between mb-2">
									<div className="flex items-center">
										{getTypeIcon(dependency.type)}
										<h4 className="font-medium text-foreground ml-2">
											{dependency.name}
										</h4>
									</div>
									<StatusBadge status={dependency.status} />
								</div>
								<Badge
									variant="outline"
									className="bg-muted/50 text-muted-foreground border-muted capitalize"
								>
									{dependency.type}
								</Badge>
							</div>
						))}
						{resources.dependsOn.length === 0 && (
							<p className="text-sm text-muted-foreground text-center py-4">
								No dependencies
							</p>
						)}
					</div>
				</CardContent>
			</Card>
		</div>
	);
}
