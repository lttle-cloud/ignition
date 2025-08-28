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
		avatar: "/avatar.png",
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
			url: "/machines",
			icon: Server,
			isActive: true,
			items: [
				{
					title: "All Machines",
					url: "/machines",
				},
			],
		},
		{
			title: "Services",
			url: "/services",
			icon: Boxes,
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
			url: "/",
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
