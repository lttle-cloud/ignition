/** biome-ignore-all lint/correctness/useExhaustiveDependencies: <explanation> */
/** biome-ignore-all lint/correctness/useUniqueElementIds: <explanation> */
import { useState, useEffect, useRef } from "react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Switch } from "@/components/ui/switch";
import {
	AlertTriangle,
	Info,
	AlertCircle,
	Bug,
	Play,
	Pause,
	Terminal,
	RefreshCw,
} from "lucide-react";

interface LogEntry {
	timestamp: string;
	level: "info" | "warn" | "error" | "debug";
	message: string;
}

interface LogsViewerProps {
	logs: LogEntry[];
}

export function LogsViewer({ logs: initialLogs }: LogsViewerProps) {
	const [logs, setLogs] = useState(initialLogs);
	const [isFollowing, setIsFollowing] = useState(false);
	const [isRefreshing, setIsRefreshing] = useState(false);
	const scrollAreaRef = useRef<HTMLDivElement>(null);
	const bottomRef = useRef<HTMLDivElement>(null);

	const getLevelConfig = (level: LogEntry["level"]) => {
		switch (level) {
			case "error":
				return {
					icon: <AlertCircle className="w-3 h-3" />,
					className: "bg-red-2/20 text-red-3 border-red-2/30",
					textColor: "text-red-3",
					badge: "ERROR",
				};
			case "warn":
				return {
					icon: <AlertTriangle className="w-3 h-3" />,
					className: "bg-orange-2/20 text-orange-3 border-orange-2/30",
					textColor: "text-orange-3",
					badge: "WARN",
				};
			case "debug":
				return {
					icon: <Bug className="w-3 h-3" />,
					className: "bg-muted/50 text-muted-foreground border-muted",
					textColor: "text-muted-foreground",
					badge: "DEBUG",
				};
			default:
				return {
					icon: <Info className="w-3 h-3" />,
					className: "bg-green-2/20 text-green-3 border-green-2/30",
					textColor: "text-green-3",
					badge: "INFO",
				};
		}
	};

	const formatTimestamp = (timestamp: string) => {
		const date = new Date(timestamp);
		const now = new Date();
		const diffMs = now.getTime() - date.getTime();
		const diffSeconds = Math.floor(diffMs / 1000);
		const diffMinutes = Math.floor(diffSeconds / 60);
		const diffHours = Math.floor(diffMinutes / 60);

		// Show relative time for recent logs, absolute for older ones
		if (diffSeconds < 60) {
			return `${diffSeconds}s ago`;
		} else if (diffMinutes < 60) {
			return `${diffMinutes}m ago`;
		} else if (diffHours < 24) {
			return `${diffHours}h ago`;
		} else {
			return date.toLocaleString("en-US", {
				month: "short",
				day: "numeric",
				hour: "2-digit",
				minute: "2-digit",
				second: "2-digit",
			});
		}
	};

	const generateNewLog = () => {
		const levels: LogEntry["level"][] = ["info", "warn", "error", "debug"];
		const messages = [
			"Processing incoming request",
			"Database connection established",
			"Cache miss for key: user_session_abc123",
			"Memory usage: 78%",
			"Failed to connect to external API, retrying...",
			"User authentication successful",
			"Background job completed successfully",
			"Rate limit exceeded for IP 192.168.1.100",
			"Configuration reloaded from environment",
			"Health check passed",
		];

		return {
			timestamp: new Date().toISOString(),
			level: levels[Math.floor(Math.random() * levels.length)],
			message: messages[Math.floor(Math.random() * messages.length)],
		};
	};

	const scrollToBottom = () => {
		bottomRef.current?.scrollIntoView({ behavior: "smooth" });
	};

	const handleRefresh = () => {
		setIsRefreshing(true);
		// Simulate refresh delay
		setTimeout(() => {
			const newLog = generateNewLog();
			setLogs((prev) => [newLog, ...prev.slice(0, 49)]); // Keep last 50 logs
			setIsRefreshing(false);
		}, 500);
	};

	useEffect(() => {
		let interval: NodeJS.Timeout;

		if (isFollowing) {
			interval = setInterval(() => {
				const newLog = generateNewLog();
				setLogs((prev) => [newLog, ...prev.slice(0, 49)]); // Keep last 50 logs
			}, 2000); // New log every 2 seconds
		}

		return () => {
			if (interval) clearInterval(interval);
		};
	}, [isFollowing, generateNewLog]);

	useEffect(() => {
		if (isFollowing) {
			scrollToBottom();
		}
	}, [isFollowing, scrollToBottom]);

	return (
		<Card className="bg-gradient-card border-border shadow-card">
			<CardHeader>
				<div className="flex items-center justify-between">
					<CardTitle className="text-xl font-bold flex items-center">
						<Terminal className="w-5 h-5 mr-2 text-primary" />
						Logs
					</CardTitle>
					<div className="flex items-center space-x-4">
						<div className="flex items-center space-x-2">
							<Switch
								id="follow-logs"
								checked={isFollowing}
								onCheckedChange={setIsFollowing}
							/>
							<label
								htmlFor="follow-logs"
								className="text-sm font-medium text-foreground cursor-pointer flex items-center"
							>
								{isFollowing ? (
									<Pause className="w-4 h-4 mr-1" />
								) : (
									<Play className="w-4 h-4 mr-1" />
								)}
								Follow logs
							</label>
						</div>
						<Button
							variant="outline"
							size="sm"
							onClick={handleRefresh}
							disabled={isRefreshing}
							className="hover:bg-primary/10 hover:text-primary hover:border-primary/30"
						>
							<RefreshCw
								className={`w-4 h-4 mr-2 ${isRefreshing ? "animate-spin" : ""}`}
							/>
							Refresh
						</Button>
					</div>
				</div>
			</CardHeader>
			<CardContent className="p-0">
				<ScrollArea ref={scrollAreaRef} className="h-96 w-full">
					<div className="bg-background/30 border-t border-border">
						{logs.map((log, index) => {
							const config = getLevelConfig(log.level);
							return (
								<div
									key={`${log.timestamp}-${index}`}
									className="flex items-start gap-3 px-4 py-2 hover:bg-muted/20 transition-colors border-b border-border/50 text-sm"
								>
									{/* Timestamp */}
									<div className="flex-shrink-0 w-20 text-xs text-muted-foreground font-mono">
										{formatTimestamp(log.timestamp)}
									</div>

									{/* Log Level */}
									<div className="flex-shrink-0">
										<Badge
											variant="outline"
											className={`${config.className} text-xs px-1.5 py-0.5 font-mono font-semibold`}
										>
											{config.badge}
										</Badge>
									</div>

									{/* Message */}
									<div className="flex-1 min-w-0">
										<p className="text-foreground font-mono text-sm leading-relaxed break-words">
											{log.message}
										</p>
									</div>
								</div>
							);
						})}
						<div ref={bottomRef} />
					</div>
				</ScrollArea>

				{logs.length === 0 && (
					<div className="text-center py-8 text-muted-foreground">
						<Terminal className="w-8 h-8 mx-auto mb-2 text-muted-foreground/50" />
						No logs available
					</div>
				)}

				{/* Status bar */}
				<div className="flex items-center justify-between px-4 py-2 bg-muted/20 border-t border-border text-xs text-muted-foreground">
					<span>{logs.length} logs loaded</span>
					{isFollowing && (
						<div className="flex items-center">
							<div className="w-2 h-2 bg-status-ready rounded-full mr-2 animate-pulse" />
							<span>Live streaming</span>
						</div>
					)}
				</div>
			</CardContent>
		</Card>
	);
}
