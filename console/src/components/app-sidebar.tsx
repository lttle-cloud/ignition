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

// Platform navigation data
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
			title: "Services",
			url: "/services",
			icon: Boxes,
			isActive: true,
			items: [
				{
					title: "All Services",
					url: "/services",
				},
				{
					title: "Create Service",
					url: "/services/new",
				},
			],
		},
		{
			title: "Machines",
			url: "/machines",
			icon: Server,
			items: [
				{
					title: "All Machines",
					url: "/machines",
				},
				{
					title: "Create Machine",
					url: "/machines/new",
				},
				{
					title: "Snapshots",
					url: "/machines/snapshots",
				},
			],
		},
		{
			title: "Volumes",
			url: "/volumes",
			icon: HardDrive,
			items: [
				{
					title: "All Volumes",
					url: "/volumes",
				},
				{
					title: "Create Volume",
					url: "/volumes/new",
				},
			],
		},
		{
			title: "Networking",
			url: "/networking",
			icon: BadgeCheck,
			items: [
				{
					title: "Domains",
					url: "/networking/domains",
				},
				{
					title: "Certificates",
					url: "/networking/certificates",
				},
			],
		},
	],
	projects: [
		{
			name: "Applications",
			url: "/applications",
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
