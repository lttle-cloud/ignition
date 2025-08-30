import type { Machine, MachineDetails } from "@/types/machine";

export const mockMachines: Machine[] = [
	{
		id: "nginx-001",
		name: "nginx",
		namespace: "default",
		mode: "regular",
		status: "ready",
		image: "docker.io/library/nginx:latest@sha256...",
		cpus: 1,
		memory: "128 MiB",
		lastBootTime: "178ms",
		createdAt: "2024-01-15T10:30:00Z",
		uptime: "3d 12h",
	},
	{
		id: "redis-001",
		name: "redis-cache",
		namespace: "production",
		mode: "regular",
		status: "ready",
		image: "docker.io/library/redis:7-alpine",
		cpus: 2,
		memory: "256 MiB",
		lastBootTime: "89ms",
		createdAt: "2024-01-14T08:15:00Z",
		uptime: "4d 8h",
	},
	{
		id: "postgres-001",
		name: "postgres-db",
		namespace: "production",
		mode: "flash",
		status: "ready",
		image: "docker.io/library/postgres:15",
		cpus: 4,
		memory: "1 GiB",
		lastBootTime: "1.2s",
		createdAt: "2024-01-12T14:22:00Z",
		uptime: "6d 18h",
	},
	{
		id: "api-001",
		name: "api-server",
		namespace: "staging",
		mode: "regular",
		status: "pending",
		image: "registry.company.com/api:v2.1.0",
		cpus: 2,
		memory: "512 MiB",
		lastBootTime: "245ms",
		createdAt: "2024-01-16T09:45:00Z",
		uptime: "2d 6h",
	},
	{
		id: "worker-001",
		name: "background-worker",
		namespace: "production",
		mode: "flash",
		status: "error",
		image: "registry.company.com/worker:latest",
		cpus: 1,
		memory: "256 MiB",
		lastBootTime: "156ms",
		createdAt: "2024-01-16T11:00:00Z",
		uptime: "1d 23h",
	},
];

export const getMachineDetails = (machineId: string): MachineDetails | null => {
	const machine = mockMachines.find((m) => m.id === machineId);
	if (!machine) return null;

	// Generate realistic time series data
	const generateHistory = (
		points: number,
		baseValue: number,
		variance: number,
	) => {
		const now = new Date();
		return Array.from({ length: points }, (_, i) => {
			const time = new Date(now.getTime() - (points - i) * 60000).toISOString();
			const value = Math.max(
				0,
				Math.min(100, baseValue + (Math.random() - 0.5) * variance),
			);
			return { time, value };
		});
	};

	return {
		machine,
		metrics: {
			cpu: {
				usage: Math.floor(Math.random() * 80) + 10,
				history: generateHistory(30, 45, 30),
			},
			memory: {
				used: machine.memory,
				total: machine.memory.includes("GiB") ? machine.memory : "512 MiB",
				percentage: Math.floor(Math.random() * 60) + 20,
				history: generateHistory(30, 40, 25),
			},
			diskIO: {
				read: `${Math.floor(Math.random() * 50) + 5} MB/s`,
				write: `${Math.floor(Math.random() * 30) + 2} MB/s`,
				history: Array.from({ length: 30 }, (_, i) => ({
					time: new Date(Date.now() - (30 - i) * 60000).toISOString(),
					read: Math.floor(Math.random() * 50) + 5,
					write: Math.floor(Math.random() * 30) + 2,
				})),
			},
			network: {
				bytesIn: `${Math.floor(Math.random() * 100) + 10} MB/s`,
				bytesOut: `${Math.floor(Math.random() * 80) + 5} MB/s`,
				history: Array.from({ length: 30 }, (_, i) => ({
					time: new Date(Date.now() - (30 - i) * 60000).toISOString(),
					in: Math.floor(Math.random() * 100) + 10,
					out: Math.floor(Math.random() * 80) + 5,
				})),
			},
			flashLocks: Math.floor(Math.random() * 5),
			bootTimeEvolution: Array.from({ length: 10 }, (_, i) => ({
				time: new Date(Date.now() - (10 - i) * 86400000).toISOString(),
				duration: Math.floor(Math.random() * 200) + 100,
			})),
		},
		resources: {
			boundVolumes: [
				{
					name: "data-volume",
					mountPath: "/var/lib/data",
					size: "10 GiB",
					type: "persistent",
				},
				{
					name: "temp-storage",
					mountPath: "/tmp",
					size: "1 GiB",
					type: "ephemeral",
				},
			],
			boundServices: [
				{
					name: "web-service",
					port: 80,
					protocol: "HTTP",
					status: "healthy",
				},
				{
					name: "api-service",
					port: 8080,
					protocol: "HTTPS",
					status: "healthy",
				},
			],
			dependsOn: [
				{
					name: "postgres-db",
					type: "machine",
					status: "ready",
				},
				{
					name: "redis-cache",
					type: "machine",
					status: "ready",
				},
			],
		},
		logs: [
			{
				timestamp: new Date(Date.now() - 1000).toISOString(),
				level: "info",
				message: "Application started successfully",
			},
			{
				timestamp: new Date(Date.now() - 30000).toISOString(),
				level: "warn",
				message: "High memory usage detected: 85%",
			},
			{
				timestamp: new Date(Date.now() - 60000).toISOString(),
				level: "info",
				message: "Connected to database",
			},
			{
				timestamp: new Date(Date.now() - 120000).toISOString(),
				level: "debug",
				message: "Configuration loaded from /etc/app/config.yml",
			},
			{
				timestamp: new Date(Date.now() - 180000).toISOString(),
				level: "error",
				message: "Failed to connect to external service, retrying in 30s",
			},
		],
	};
};
