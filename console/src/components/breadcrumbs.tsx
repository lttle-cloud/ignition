import { isMatch, Link, useMatches } from "@tanstack/react-router";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";

export const Breadcrumbs = () => {
	const matches = useMatches();

	if (matches.some((match) => match.status === "pending")) return null;

	const matchesWithCrumbs = matches.filter((match) =>
		isMatch(match, "loaderData.crumb"),
	);

	return (
		<Breadcrumb>
			<BreadcrumbList>
				{matchesWithCrumbs.map((match, i) => (
					<>
						<BreadcrumbItem className="hidden md:block" key={match.id}>
							{i + 1 === matchesWithCrumbs.length ? (
								<BreadcrumbPage>{match.loaderData?.crumb}</BreadcrumbPage>
							) : (
								<BreadcrumbLink>
									<Link from={match.fullPath}>{match.loaderData?.crumb}</Link>
								</BreadcrumbLink>
							)}
						</BreadcrumbItem>
						{i + 1 < matchesWithCrumbs.length ? (
							<BreadcrumbSeparator className="hidden md:block" />
						) : null}
					</>
				))}
			</BreadcrumbList>
		</Breadcrumb>
	);
};
