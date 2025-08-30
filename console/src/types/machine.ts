export interface Machine {
	id: string;
	name: string;
	namespace: string;
	mode: "regular" | "flash";
	status: "ready" | "pending" | "error" | "warning";
	image: string;
	cpus: number;
	memory: string;
	lastBootTime: string;
	createdAt: string;
	uptime: string;
}

export interface MachineMetrics {
	cpu: {
		usage: number;
		history: Array<{ time: string; value: number }>;
	};
	memory: {
		used: string;
		total: string;
		percentage: number;
		history: Array<{ time: string; value: number }>;
	};
	diskIO: {
		read: string;
		write: string;
		history: Array<{ time: string; read: number; write: number }>;
	};
	network: {
		bytesIn: string;
		bytesOut: string;
		history: Array<{ time: string; in: number; out: number }>;
	};
	flashLocks: number;
	bootTimeEvolution: Array<{ time: string; duration: number }>;
}

export interface MachineResources {
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
}

export interface MachineDetails {
	machine: Machine;
	metrics: MachineMetrics;
	resources: MachineResources;
	logs: Array<{
		timestamp: string;
		level: "info" | "warn" | "error" | "debug";
		message: string;
	}>;
}
