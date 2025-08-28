"use client";

import { BadgeCheck, Boxes, Frame, HardDrive, Server } from "lucide-react";
import type * as React from "react";

import { NavMain } from "@/components/nav-main";
import { NavProjects } from "@/components/nav-projects";
import { NavUser } from "@/components/nav-user";
import { TenantSwitcher } from "@/components/tenant-switcher";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarHeader,
	SidebarRail,
} from "@/components/ui/sidebar";

// This is sample data.
const data = {
	user: {
		name: "Stefan Ghegoiu",
		email: "stefan@lttle.cloud",
		avatar: "/public/avatar.png",
	},
	tenants: [
		{
			name: "Acme Inc",
		},
		{
			name: "Acme Corp.",
		},
		{
			name: "Evil Corp.",
		},
	],
	navMain: [
		{
			title: "Machines",
			url: "#",
			icon: Server,
			isActive: true,
			items: [
				{
					title: "History",
					url: "#",
				},
				{
					title: "Starred",
					url: "#",
				},
				{
					title: "Settings",
					url: "#",
				},
			],
		},
		{
			title: "Volumes",
			url: "#",
			icon: HardDrive,
			items: [
				{
					title: "Genesis",
					url: "#",
				},
				{
					title: "Explorer",
					url: "#",
				},
				{
					title: "Quantum",
					url: "#",
				},
			],
		},
		{
			title: "Services",
			url: "#",
			icon: Boxes,
			items: [
				{
					title: "Introduction",
					url: "#",
				},
				{
					title: "Get Started",
					url: "#",
				},
				{
					title: "Tutorials",
					url: "#",
				},
				{
					title: "Changelog",
					url: "#",
				},
			],
		},
		{
			title: "Certificates",
			url: "#",
			icon: BadgeCheck,
			items: [
				{
					title: "General",
					url: "#",
				},
				{
					title: "Team",
					url: "#",
				},
				{
					title: "Billing",
					url: "#",
				},
				{
					title: "Limits",
					url: "#",
				},
			],
		},
	],
	projects: [
		{
			name: "Deployments",
			url: "#",
			icon: Frame,
		},
	],
};

export function AppSidebar({ ...props }: React.ComponentProps<typeof Sidebar>) {
	return (
		<Sidebar collapsible="icon" {...props}>
			<SidebarHeader>
				<TenantSwitcher tenants={data.tenants} />
			</SidebarHeader>
			<SidebarContent>
				<NavProjects projects={data.projects} />
				<NavMain items={data.navMain} />
			</SidebarContent>
			<SidebarFooter>
				<NavUser user={data.user} />
			</SidebarFooter>
			<SidebarRail />
		</Sidebar>
	);
}
