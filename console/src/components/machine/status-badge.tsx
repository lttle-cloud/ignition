import { cn } from "@/lib/utils";

interface StatusBadgeProps {
	status: "ready" | "pending" | "error" | "warning";
	className?: string;
}

export function StatusBadge({ status, className }: StatusBadgeProps) {
	const statusConfig = {
		ready: {
			label: "Ready",
			bg: "bg-green-2/20",
			text: "text-green-3",
			border: "border-green-2/30",
			dot: "bg-green-2",
		},
		pending: {
			label: "Pending",
			bg: "bg-yellow-2/20",
			text: "text-yellow-3",
			border: "border-yellow-2/30",
			dot: "bg-yellow-2",
		},
		error: {
			label: "Error",
			bg: "bg-red-2/20",
			text: "text-red-3",
			border: "border-red-2/30",
			dot: "bg-red-2",
		},
		warning: {
			label: "Warning",
			bg: "bg-orange-2/20",
			text: "text-orange-3",
			border: "border-orange-2/30",
			dot: "bg-orange-2",
		},
	};

	const config = statusConfig[status];

	return (
		<span
			className={cn(
				"inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium border transition-colors",
				config.bg,
				config.text,
				config.border,
				className,
			)}
		>
			<span className={cn("w-1.5 h-1.5 rounded-full mr-1.5", config.dot)} />
			{config.label}
		</span>
	);
}
